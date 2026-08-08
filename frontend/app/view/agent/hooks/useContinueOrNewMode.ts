// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Continue / New view-mode logic for AgentLaunchModal — Feature A,
 * SPEC_AGENT_LAUNCH_AND_MODAL_DISMISSAL §A. When the current
 * AgentDefinition has past instances, the modal defaults to "Continue"
 * (most-recent instance preselected) so re-opening a configured agent
 * is one step instead of a dropdown hunt; a toggle drops to the full
 * "New agent" form.
 *
 * Extracted out of AgentLaunchModal.tsx (modularization pass,
 * 2026-07-23) — this hook owns everything about *which* past instance
 * (if any) the form is continuing, and the New/Continue toggle itself.
 * It does NOT own form-field values (name/runtime/image/…) — those
 * stay in the caller-supplied `flow` store, which this hook reads and
 * dispatches into.
 */

import { createEffect, createMemo, createResource, createSignal } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    continueLocksIdentity as flowContinueLocksIdentity,
    continueLocksMemory as flowContinueLocksMemory,
    type LaunchFlowStore,
} from "@/app/store/launch-flow-state";
import { realAccountIdOrEmpty } from "@/app/view/agent/identity-carry-over";
import { refreshAccountCache } from "@/app/view/identity/identity-model";

export interface UseContinueOrNewModeOpts {
    /** The launch-flow reactive store — pass BY REFERENCE (the whole
     *  object), never destructure its fields at the call site. `flow`
     *  is threaded through nearly everything in the modal and reads
     *  here must stay tracked against the live store, not a snapshot. */
    flow: LaunchFlowStore;
    /** Definition id — scopes the "Continue an existing agent" list to
     *  past launches of THIS definition. */
    agentId: string;
    /** True when the modal opened from a restored form snapshot (the
     *  "+ New bundle" create → replace-back round-trip). When true,
     *  `initialContinueOfId` decides the starting view mode instead of
     *  the most-recent-instance heuristic firing once `namedAgents`
     *  resolves. */
    hasInitialFormState: boolean;
    /** `props.initialFormState?.continueOfId` — non-null means the user
     *  was in Continue mode when they left for the round-trip. */
    initialContinueOfId?: string | null;
}

export function useContinueOrNewMode(opts: UseContinueOrNewModeOpts) {
    const { flow } = opts;

    // v8 — "Continue agent" dropdown. Filters to instances of the
    // CURRENT definition (server-side; a global cap would let older
    // rows of this definition fall off when users have many agents
    // across definitions). Empty list = no past launches for this
    // definition, dropdown hides itself.
    const [namedAgents] = createResource<NamedAgentRow[]>(async () => {
        try {
            return await RpcApi.ListNamedAgentsCommand(TabRpcClient, {
                limit: 200,
                definition_id: opts.agentId,
            });
        } catch {
            return [];
        }
    });

    /** "" = "— New agent —" (default). Non-empty = continuing that
     *  past instance; name + identity + memory are pre-filled and
     *  locked. Mirrors the store's `form.continueOfId` (which uses
     *  `null` instead of "" — the dropdown uses "" as its UI sentinel
     *  for the placeholder option). */
    const continueOfId = () => flow.state.form.continueOfId ?? "";

    const continuedRow = createMemo(() => {
        const id = continueOfId();
        if (!id) return null;
        return (namedAgents() ?? []).find((r) => r.instance_id === id) ?? null;
    });

    // Continuation status is read from the reducer's `form.continueOfId`
    // (the source of truth) rather than from `continuedRow()` — the
    // latter depends on `namedAgents()` having loaded, so on round-trip
    // re-open isContinue() would transiently be false until that
    // resource lands, flipping the auth gate on for a tick. Reading the
    // form directly keeps the answer stable from the moment Opened
    // dispatches with a restored continueOfId.
    const isContinue = () => flow.state.form.continueOfId !== null;

    // Per-bundle continuation locks come from the slice's selectors.
    // Local memos read flow.state.form so they invalidate when the
    // selection or carry-over identity changes.
    const continueLocksIdentity = createMemo(() => flowContinueLocksIdentity(flow.state));
    const continueLocksMemory = createMemo(() => flowContinueLocksMemory(flow.state));

    // Sequencing guard (reagentx P1 on #2464): handleContinueSelect awaits
    // an RPC round-trip (refreshAccountCache) before dispatching, which it
    // didn't when this was fully synchronous. Without this, two calls in
    // flight at once (e.g. the user changes the dropdown selection again
    // before the first fetch resolves) could resolve out of order, and a
    // stale call finishing last would dispatch over a newer selection —
    // silently locking the form to the wrong prior instance's account
    // while the UI shows a different one. Only the most-recently-STARTED
    // call is allowed to dispatch; every earlier in-flight call drops its
    // result silently once superseded.
    let continueSelectSeq = 0;
    const handleContinueSelect = async (rawId: string) => {
        const seq = ++continueSelectSeq;
        const id = rawId === "" ? null : rawId;
        const row =
            id === null
                ? null
                : (namedAgents() ?? []).find((r) => r.instance_id === id) ?? null;
        // Legacy rows may carry "" or "blank" identity_id from before the
        // blank-removal, or (pre-issue-#1624-PR-C rows) a legacy sentinel
        // like "default" that no longer resolves to a real account. Treat
        // all of these as "no carry-over" so the user must pick a real
        // account for the continuation — an unresolvable carried id
        // already falls back to "re-pick" here rather than needing a
        // backend resolution step. Forwarding one of these as accountId
        // causes a real FOREIGN KEY failure in linkagentidentity (see
        // identity-carry-over.ts's realAccountIdOrEmpty). A UUID-shape
        // check alone isn't enough — a pre-#1624-PR-C identity-bundle id
        // was also UUID-formatted, just not an account id — so this cross-
        // checks against a fresh account fetch (reagentx P2 on #2464,
        // flagging this call site was still on the shape-only check while
        // AgentPicker.tsx's sibling call sites got the stronger one).
        //
        // memoryId intentionally does NOT get the same treatment: unlike
        // account_id, memory_id has no FK constraint, and legitimate
        // bundle ids are routinely non-UUID ("blank", "seed-*" —
        // memory_bundles.rs/bundle.rs) rather than legacy garbage — a
        // UUID-shape filter here would silently drop a real carry-over
        // (reagentx P2 on #2464, which found the same mistake newly
        // introduced elsewhere).
        const carry = row
            ? {
                  name: row.instance_name,
                  accountId: realAccountIdOrEmpty(row.identity_id, (await refreshAccountCache()).map((a) => a.id)),
                  memoryId: row.memory_id,
              }
            : undefined;
        if (seq !== continueSelectSeq) return; // superseded by a later call — drop this stale dispatch
        flow.dispatch({ type: "ContinueOfChanged", continueOfId: id, carry });
    };

    const mostRecentInstance = (): NamedAgentRow | null => {
        const rows = namedAgents() ?? [];
        if (rows.length === 0) return null;
        return [...rows].sort((a, b) => (b.started_at ?? 0) - (a.started_at ?? 0))[0];
    };

    // Initial viewMode honors a restored continuation: if the snapshot
    // captured a `continueOfId`, the user was in Continue mode when
    // they left for the `+ New bundle` flow — restore them there. The
    // previous "+New buttons disabled while continuing → always restore
    // as New" assumption was false for ambient-creds continuations
    // (continueLocksIdentity is false when the carried identity is
    // empty, so the +New button stays enabled even in Continue mode).
    const [viewMode, setViewMode] = createSignal<"continue" | "new">(
        opts.initialContinueOfId != null ? "continue" : "new",
    );
    // viewModeDecided suppresses the auto-decide effect when we're
    // restoring from a snapshot — the snapshot's continueOfId decides
    // it, not the most-recent-instance heuristic.
    let viewModeDecided = opts.hasInitialFormState;
    createEffect(() => {
        const rows = namedAgents();
        if (rows === undefined || viewModeDecided) return;
        viewModeDecided = true;
        const recent = mostRecentInstance();
        if (recent) {
            setViewMode("continue");
            handleContinueSelect(recent.instance_id);
        }
    });

    const enterNewMode = () => {
        setViewMode("new");
        handleContinueSelect(""); // clears continueOfId; unlocks identity/memory
    };
    const enterContinueMode = () => {
        setViewMode("continue");
        if (continueOfId()) return;
        const recent = mostRecentInstance();
        if (recent) handleContinueSelect(recent.instance_id);
    };

    return {
        namedAgents,
        continueOfId,
        continuedRow,
        isContinue,
        continueLocksIdentity,
        continueLocksMemory,
        handleContinueSelect,
        viewMode,
        enterNewMode,
        enterContinueMode,
    };
}
