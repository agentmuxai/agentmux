// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the Claude sign-in does once the login reports an outcome: confirm the link, clean up the stale alias, show the result. */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { refreshAccountCache } from "@/app/view/identity/identity-model";
import type { ProviderLoginOutcome } from "@/app/view/agent/flows/run-provider-login";
import { CLAUDE_PROVIDER } from "./claude-login-model";
import type { LoginState } from "./claude-login-state";

/**
 * Whether the account is really linked to the agent. finalizeAccount (run-provider-login.ts) catches and logs
 * — does NOT rethrow — a failed LinkAgentIdentityCommand, so onAccountRegistered firing only proves the ACCOUNT
 * was persisted, not that it's linked. For a linkTarget flow that's the whole point of the click, so confirm
 * the link is there before showing success, instead of "Signed in" for an agent whose next spawn the
 * resolver's gate still blocks (#2414).
 */
export async function confirmLink(s: LoginState): Promise<boolean> {
    const target = s.props.linkTarget;
    if (!target?.agentDefinitionId) return true;
    try {
        const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, {
            agent_id: target.agentDefinitionId,
        });
        // Must check the CANONICAL "claude" provider specifically, not just
        // account_id. A failed link insert is only warned about, never
        // rethrown — for a re-login on a legacy-aliased row, registeredAccountId
        // equals that SAME alias row's own account_id (it's a refresh, not a
        // fresh mint), so a bare account_id match is vacuously satisfied by the
        // pre-existing alias link even when the new canonical insert silently
        // failed. That false "confirmed" then fires the staleAliasProvider
        // unlink, deleting the one link that actually worked — leaving the
        // agent with ZERO identity links, worse than the orphaned-alias bug
        // this panel exists to fix.
        return links.some((l) => l.account_id === s.registeredAccountId && l.provider === CLAUDE_PROVIDER.id);
    } catch (e) {
        console.warn(`[claude-login] failed to verify the agent link: ${(e as Error)?.message ?? String(e)}`);
        return false;
    }
}

/**
 * See `staleAliasProvider`: the link just confirmed used the canonical "claude" key, never the alias — clean up
 * the orphaned alias row now so it can't abort a future spawn. `silent: true`: a plain unlink publishes
 * agentcredentials:revoked, which the agent pane shows as "Credentials revoked" — misleading right after a
 * successful re-login where the credential and effective binding are both fine; this is a migration, not a
 * real unbind. Best-effort either way: the new canonical link already succeeded and is what matters.
 */
function unlinkStaleAlias(s: LoginState): void {
    const { staleAliasProvider, linkTarget } = s.props;
    if (!staleAliasProvider || !linkTarget?.agentDefinitionId) return;
    RpcApi.UnlinkAgentIdentityCommand(TabRpcClient, {
        agent_id: linkTarget.agentDefinitionId,
        provider: staleAliasProvider,
        silent: true,
    }).catch((e) => {
        console.warn(`[claude-login] failed to unlink stale alias "${staleAliasProvider}": ${e?.message ?? String(e)}`);
    });
}

/** Whether this outcome is a sign-in that has to be confirmed against the agent it was started for. */
export function needsLinkCheck(s: LoginState, outcome: ProviderLoginOutcome): boolean {
    const signedIn = outcome === "inapp-success" || outcome === "terminal-success";
    return signedIn && !!s.registeredAccountId && !!s.props.linkTarget?.agentDefinitionId;
}

function finishSignedIn(s: LoginState, linkConfirmed: boolean): void {
    const accountId = s.registeredAccountId;
    if (!accountId) {
        s.setError("Login completed but the account couldn't be registered. Try again.");
        return;
    }
    if (!linkConfirmed) {
        s.setError("Signed in, but AgentMux couldn't confirm the account was linked to this agent. Try again.");
        return;
    }
    void refreshAccountCache();
    unlinkStaleAlias(s);
    s.setDone(true);
    s.props.onSignedIn?.(accountId);
}

/**
 * Show an outcome: signed in, or why not. Synchronous, so that it lands in the same turn as the flow's
 * `finally` (the panel is not "in flight" once it shows a result); the one thing that has to be waited for, the
 * link check, is `confirmLink`, before this.
 */
export function applyOutcome(s: LoginState, outcome: ProviderLoginOutcome, linkConfirmed: boolean): void {
    switch (outcome) {
        case "inapp-success":
        case "terminal-success":
            finishSignedIn(s, linkConfirmed);
            break;
        case "inapp-timeout":
            s.setError("The login link timed out. Complete it in your browser, then try again.");
            break;
        case "inapp-ended":
            s.setError("The sign-in ended before it finished. Try again; if it asks for a code, paste the whole code.");
            break;
        case "terminal-timeout":
            s.setError("Opened a terminal window, but no login was detected within 5 minutes.");
            break;
        case "terminal-unavailable":
            s.setError("Couldn't start a browser login or open a terminal window on this platform.");
            break;
    }
}
