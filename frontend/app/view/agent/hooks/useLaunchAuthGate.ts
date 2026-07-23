// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Auth-gating logic for AgentLaunchModal (spec:
 * SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md). Decides whether Launch
 * must be blocked pending OAuth, whether a soft "reconnect" nudge
 * should show, and derives the account-status memos both of those
 * depend on.
 *
 * Extracted out of AgentLaunchModal.tsx (modularization pass,
 * 2026-07-23). Pure derivation over the shared `flow` store plus two
 * caller-supplied accessors (`provider`, `isContinue`) — no owned
 * state of its own.
 */

import { createMemo } from "solid-js";

import { accountSuppliesProvider, type LaunchFlowStore } from "@/app/store/launch-flow-state";
import type { ProviderDefinition } from "../providers";

export interface UseLaunchAuthGateOpts {
    /** The launch-flow reactive store — pass BY REFERENCE (the whole
     *  object), never destructure its fields at the call site, so
     *  reads here stay tracked against the live store. */
    flow: LaunchFlowStore;
    /** The agent definition's resolved provider config (may be
     *  undefined transiently before the catalog memo settles). */
    provider: () => ProviderDefinition | undefined;
    /** Whether this launch is continuing a prior named agent — prior
     *  launches already produced creds, so the auth gate never
     *  applies to a continuation. */
    isContinue: () => boolean;
}

export function useLaunchAuthGate(opts: UseLaunchAuthGateOpts) {
    const { flow, provider, isContinue } = opts;

    // True when the selected account actually supplies credentials for
    // the agent's provider. Replaces `bundleHasMatchingBinding` — with
    // a direct account selection instead of a bundle-of-bindings, this
    // is a plain lookup against the already-loaded account list (issue
    // #1624 PR-C Part B).
    const accountSupplies = createMemo(() =>
        accountSuppliesProvider(flow.state, provider()?.id ?? ""),
    );

    // The selected account's own `status` field. Replaces
    // `bundleBindingStatus` — no bundle→binding→account join needed
    // anymore, `flow.state.accounts.list` is already reactive (updated
    // by the caller's account-cache refresh), so no separate cache-tick
    // signal is needed either.
    const selectedAccountStatus = createMemo<string | null>(() => {
        const id = flow.state.form.accountId;
        if (!id) return null;
        return flow.state.accounts.list.find((a) => a.id === id)?.status ?? null;
    });

    // True when the selected account is in an oauth-class state that
    // benefits from a Reconnect nudge — strictly a wording trigger, not
    // a launch-blocker (spec §4.4: "wording-only nudge"). The Launch
    // button stays enabled because the account still counts; the CLI
    // will refresh on its first call.
    const accountNeedsReconnectNudge = createMemo(() => {
        const s = selectedAccountStatus();
        return s === "needs_reauth" || s === "expired";
    });

    // Auth gate applies to fresh launches of OAuth providers when the
    // selected account can't supply credentials for the agent's
    // provider. That's true when:
    //
    // - No account selected at all.
    // - A selected account for a different provider.
    //
    // Bypasses:
    // - `isContinue` — prior launch already produced creds.
    // - API-key providers (kimi/pi) — their existing `launch-flow.ts`
    //   Phase 2 prompts for the key in-line. Reagent + codex P1 on #847.
    //
    // Hard auth-blockers: launch CANNOT proceed without the user
    // completing OAuth. Drives both the panel mount AND the launch
    // gate.
    //
    // 2026-07-20: reverted a same-day "no account = ambient creds is fine"
    // relaxation. `identity/resolver.rs`'s layer-3 spawn gate was ALREADY
    // hard-blocking an oauth-class agent with no bound account by default
    // (`use_ambient_login=0`) — that relaxation let Launch enable and then
    // had the agent fail its first real turn with a raw backend error,
    // which is worse than being blocked up front with a clear reason. The
    // gate's ambient escape hatch is now removed entirely (single point,
    // not global — PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7),
    // so "no account selected" must block here again, for every provider,
    // with no exception.
    const authBlocksLaunch = () =>
        !isContinue()
        && provider()?.authType === "oauth"
        && (flow.state.form.accountId === "" || !accountSupplies());
    // Soft nudges: show the Connect CTA (with status-aware wording)
    // when the account is selected but its status is `needs_reauth` /
    // `expired`. Per spec §4.4 this is a "wording-only nudge" — does
    // NOT block Launch. The CLI's own refresh path handles the rest on
    // first call; the nudge just gives the user a one-click path to
    // refresh proactively.
    const authNeedsReconnectWording = () =>
        !isContinue()
        && provider()?.authType === "oauth"
        && accountSupplies()
        && accountNeedsReconnectNudge();
    // Mount the panel for EITHER reason (hard block or soft nudge).
    const authRequired = () => authBlocksLaunch() || authNeedsReconnectWording();
    // Launch-readiness gate. Note this only consults `authBlocksLaunch`
    // — the wording-only path doesn't gate. The panel may still show a
    // `ConnectCta` in the nudge case, but `authReady` returns true and
    // Launch is clickable.
    const authReady = () => !authBlocksLaunch() || flow.state.auth.kind === "ready";

    return {
        authBlocksLaunch,
        authNeedsReconnectWording,
        authRequired,
        authReady,
        accountSupplies,
        selectedAccountStatus,
        accountNeedsReconnectNudge,
    };
}
