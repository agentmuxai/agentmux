// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ClaudeLoginPanel — the in-app Claude/Anthropic login session, driven from
 * a context that has no agent block: the Armory Accounts gallery's "Connect"
 * (fresh account, no `linkTarget`) and the Agent Stash's per-binding
 * "Connect / Re-login" action (`linkTarget` + `existingAccountId`, refresh
 * not mint). Both reuse the exact same session logic and UI
 * (`InAppLoginPanel`) as the launch surface's `PreLaunchAuthPanel` — see
 * docs/specs/SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.3 surface 3.
 *
 * Deliberately simpler than PreLaunchAuthPanel's `AuthFlowController`: there
 * is no provider dropdown to switch away from mid-login (this panel is
 * Claude-only) and no launch sequence waiting on the result, so a plain
 * boolean in-flight guard + a few signals is enough — no action-token
 * staleness tracking needed.
 */

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { getApi } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { translateError } from "@/app/errors/translate";
import { PROVIDERS } from "@/app/view/agent/providers/catalog";
import { runProviderLogin, type ProviderLoginOutcome } from "@/app/view/agent/flows/run-provider-login";
import { InAppLoginPanel, type InAppLoginPhase } from "@/app/view/agent/components/InAppLoginPanel";
import { refreshAccountCache } from "@/app/view/identity/identity-model";

const CLAUDE_PROVIDER = PROVIDERS["claude"];

export function ClaudeLoginPanel(props: {
    onClose: () => void;
    /** Set for the Stash's per-binding re-login: refreshes THIS account's
     *  isolated dir instead of minting a new one, and links the result back
     *  to the agent. Omit for Armory's bare Connect (fresh account, no
     *  agent to link yet — the account just needs to exist for a later
     *  launch/Stash bind). */
    existingAccountId?: string;
    linkTarget?: { blockId?: string; agentDefinitionId: string };
    /** reagent P0 on PR #2414: set when the row this panel was opened from
     *  carries a legacy-alias provider string (e.g. "claude-code") — a
     *  successful login here links the CANONICAL "claude" provider
     *  (finalizeAccount → agent_identity_link `ON CONFLICT(agent_id,
     *  provider)`), which is a DIFFERENT key than the alias row, so it
     *  INSERTS a second link instead of replacing the broken one. The old
     *  alias row (pointing at a deleted/orphaned account) is left behind —
     *  the resolver's `inject.rs` iterates ALL of an agent's bindings
     *  ORDER BY provider and aborts the ENTIRE spawn on the first one that
     *  fails to resolve, so the orphaned alias row (sorting after "claude")
     *  silently blocks every future spawn even though the healthy "claude"
     *  link now exists — "✓ Signed in" but the agent stays broken. On
     *  success, this panel unlinks `staleAliasProvider` for the same
     *  agent so only the canonical row remains. */
    staleAliasProvider?: string;
}): JSX.Element {
    const [phase, setPhase] = createSignal<InAppLoginPhase>("starting");
    const [authUrl, setAuthUrl] = createSignal<string | undefined>(undefined);
    const [error, setError] = createSignal<string | null>(null);
    const [done, setDone] = createSignal(false);
    let cancelled = false;
    let terminalRequested = false;
    let inFlight = false;
    // Set by onAccountRegistered — run-provider-login.ts fires it ONLY once
    // the account row is actually persisted (reagent P0 on #2263). A
    // credential can be validly seeded/pasted on disk while that persist
    // call itself fails (REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_
    // 2026_07_27.md) — trusting the outcome string alone would show
    // "Signed in" for a login the resolver's spawn gate then blocks on the
    // agent's next spawn, with no account ever having existed. reagent P1
    // on PR #2414: this panel omitted the check both PreLaunchAuthPanel and
    // useAgentControllerStatus.ts's relogin() already make.
    let registeredAccountId: string | undefined;

    // Kill a genuinely orphaned login (in-flight up to 5 min) if the panel
    // unmounts before it resolves — otherwise the spawned CLI child and
    // this component's poll keep running detached, holding the host's
    // singleton cli_login_* state and blocking any later login attempt
    // from starting. reagent P1 on PR #2414.
    //
    // Guarded on `inFlight` (codex P2 on PR #2414): this panel can also
    // unmount AFTER reaching its success/error screen, while still counted
    // as "mounted" for a moment (e.g. the user lingers on "✓ Signed in"
    // before closing). cancelCliLogin() hits the host's SINGLE global
    // login slot — if a different window/surface has since started its
    // own login, an unconditional call here would kill that unrelated,
    // newer attempt instead of a no-op cleanup of our own already-finished
    // one.
    onCleanup(() => {
        cancelled = true;
        if (inFlight) {
            getApi().cancelCliLogin().catch(() => {});
        }
    });

    const runAttempt = async (cliPath: string, authEnv: Record<string, string>, skipTier1: boolean) =>
        runProviderLogin({
            provider: CLAUDE_PROVIDER,
            cliPath,
            authEnv,
            existingAccountId: props.existingAccountId,
            linkTarget: props.linkTarget,
            // See PreLaunchAuthPanel's identical rationale: the in-app tier 1
            // runs first; "Use terminal instead" re-runs with it skipped.
            skipTier1,
            awaitTier1Completion: true,
            setAuthUrl: (url) => {
                setAuthUrl(url ?? undefined);
                if (url) setPhase("waiting-authorize");
            },
            log: (_cat, msg) => console.log(`[claude-login] ${msg}`),
            isCancelled: () => cancelled || terminalRequested,
            onAccountRegistered: (accountId) => {
                registeredAccountId = accountId;
            },
            onTierChange: (event) => {
                if (event.tier === "inapp-waiting") setPhase("waiting-authorize");
                else if (event.tier === "fallback") setPhase("fallback");
                else setPhase("terminal-polling");
            },
        });

    const start = async () => {
        if (inFlight) return;
        inFlight = true;
        setError(null);
        setPhase("starting");
        try {
            let cliPath: string;
            try {
                const r = await RpcApi.ResolveCliCommand(
                    TabRpcClient,
                    {
                        provider_id: CLAUDE_PROVIDER.id,
                        cli_command: CLAUDE_PROVIDER.cliCommand,
                        npm_package: CLAUDE_PROVIDER.npmPackage,
                        pinned_version: CLAUDE_PROVIDER.pinnedVersion,
                        windows_install_command: CLAUDE_PROVIDER.windowsInstallCommand,
                        unix_install_command: CLAUDE_PROVIDER.unixInstallCommand,
                    },
                    { timeout: 120000 },
                );
                cliPath = r.cli_path;
            } catch (e) {
                const t = translateError(e);
                setError(`${t.title}: ${t.message}${t.retry ? ` — ${t.retry}` : ""}`);
                return;
            }
            const authEnv: Record<string, string> = {};
            if (CLAUDE_PROVIDER.authConfigDirEnvVar) {
                try {
                    authEnv[CLAUDE_PROVIDER.authConfigDirEnvVar] = await getApi().ensureAuthDir(CLAUDE_PROVIDER.id);
                } catch (e) {
                    setError((e as Error)?.message ?? String(e));
                    return;
                }
            }
            if (cancelled) return;

            let outcome: ProviderLoginOutcome = await runAttempt(cliPath, authEnv, false);
            if (outcome === "inapp-timeout" && terminalRequested && !cancelled) {
                // Same release/reset dance as PreLaunchAuthPanel's identical
                // "Use terminal instead" handling — see its comment.
                terminalRequested = false;
                setPhase("fallback");
                outcome = await runAttempt(cliPath, authEnv, true);
            }
            if (cancelled) return;
            switch (outcome) {
                case "inapp-success":
                case "seeded":
                case "terminal-success":
                    if (registeredAccountId) {
                        // codex P1 on PR #2414: finalizeAccount (run-provider-
                        // login.ts) catches and logs — does NOT rethrow — a
                        // failed LinkAgentIdentityCommand, so
                        // onAccountRegistered firing only proves the ACCOUNT
                        // was persisted, not that it's actually linked to
                        // this agent. For a Stash flow (linkTarget set),
                        // that's the whole point of the click — confirm the
                        // link is really there before showing success,
                        // instead of "Signed in" for an agent whose next
                        // spawn the resolver's gate still blocks.
                        let linkConfirmed = true;
                        if (props.linkTarget?.agentDefinitionId) {
                            linkConfirmed = false;
                            try {
                                const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, {
                                    agent_id: props.linkTarget.agentDefinitionId,
                                });
                                // reagent P0 on PR #2414 (round 3): must check
                                // the CANONICAL "claude" provider specifically,
                                // not just account_id. finalizeAccount only
                                // warns/swallows a failed link insert (never
                                // rethrows) — for a Stash re-login on a
                                // legacy-aliased row, registeredAccountId
                                // equals that SAME alias row's own account_id
                                // (it's a refresh, not a fresh mint), so a bare
                                // account_id match is vacuously satisfied by
                                // the pre-existing alias link even when the
                                // new canonical insert silently failed. That
                                // false "confirmed" then fires the
                                // staleAliasProvider unlink below, deleting
                                // the one link that actually worked — leaving
                                // the agent with ZERO identity links, worse
                                // than the pre-PR orphaned-alias bug this
                                // panel exists to fix.
                                linkConfirmed = links.some(
                                    (l) => l.account_id === registeredAccountId && l.provider === CLAUDE_PROVIDER.id,
                                );
                            } catch (e) {
                                console.warn(
                                    `[claude-login] failed to verify the agent link: ${(e as Error)?.message ?? String(e)}`,
                                );
                            }
                        }
                        if (!linkConfirmed) {
                            setError(
                                "Signed in, but AgentMux couldn't confirm the account was linked to this agent. Try again.",
                            );
                            break;
                        }
                        void refreshAccountCache();
                        // See staleAliasProvider's own doc comment: the link
                        // just confirmed above used the canonical "claude"
                        // key, never the alias — clean up the orphaned alias
                        // row now so it can't abort a future spawn.
                        // `silent: true` (codex P2 on PR #2414): a plain
                        // unlink publishes agentcredentials:revoked, which
                        // the agent pane shows as "Credentials revoked" —
                        // misleading immediately after a successful
                        // re-login where the credential and effective
                        // binding are both fine; this is a migration, not a
                        // real unbind. Best-effort either way: the new
                        // canonical link already succeeded and is what
                        // matters for "signed in".
                        if (props.staleAliasProvider && props.linkTarget?.agentDefinitionId) {
                            RpcApi.UnlinkAgentIdentityCommand(TabRpcClient, {
                                agent_id: props.linkTarget.agentDefinitionId,
                                provider: props.staleAliasProvider,
                                silent: true,
                            }).catch((e) => {
                                console.warn(
                                    `[claude-login] failed to unlink stale alias "${props.staleAliasProvider}": ${e?.message ?? String(e)}`,
                                );
                            });
                        }
                        setDone(true);
                    } else {
                        setError("Login completed but the account couldn't be registered. Try again.");
                    }
                    break;
                case "inapp-timeout":
                    setError("The login link timed out. Complete it in your browser, then try again.");
                    break;
                case "terminal-timeout":
                    setError("Opened a terminal window, but no login was detected within 5 minutes.");
                    break;
                case "terminal-unavailable":
                    setError("Couldn't start a browser login or open a terminal window on this platform.");
                    break;
            }
        } catch (e) {
            // codex P2 on PR #2414: runAttempt (runProviderLogin) can
            // REJECT outright — e.g. the PTY child fails to spawn, or an
            // IPC call errors — not just resolve to a failure outcome
            // string. This try previously had no catch, only finally, so a
            // rejection here escaped `void start()`'s fire-and-forget call
            // (line ~208) as an unhandled rejection, leaving the panel
            // stuck on its starting phase forever with no error shown and
            // no Retry button (same class of bug as PreLaunchAuthPanel's
            // identical gap, reagent P2 on PR #2410).
            getApi().cancelCliLogin().catch(() => {});
            const t = translateError(e);
            setError(`${t.title}: ${t.message}${t.retry ? ` — ${t.retry}` : ""}`);
        } finally {
            inFlight = false;
        }
    };

    const onCancel = () => {
        cancelled = true;
        getApi().cancelCliLogin().catch(() => {});
        props.onClose();
    };

    void start();

    return (
        <div
            class="accounts-chooser-overlay"
            onClick={(e) => e.target === e.currentTarget && !inFlight && props.onClose()}
        >
            <div class="accounts-chooser" role="dialog" aria-label="Connect Anthropic">
                <Show
                    when={!done()}
                    fallback={
                        <div class="accounts-chooser-modes">
                            <div class="oauth-byo-note">✓ Signed in to Claude.</div>
                            <div class="identity-key-actions">
                                <button class="identity-btn identity-btn-primary" onClick={() => props.onClose()}>
                                    Done
                                </button>
                            </div>
                        </div>
                    }
                >
                    <Show
                        when={!error()}
                        fallback={
                            <div class="accounts-chooser-modes">
                                <div class="oauth-byo-note">{error()}</div>
                                <div class="identity-key-actions">
                                    <button class="identity-btn identity-btn-primary" onClick={() => void start()}>
                                        Retry
                                    </button>
                                    <button class="identity-btn identity-btn-secondary" onClick={() => props.onClose()}>
                                        Close
                                    </button>
                                </div>
                            </div>
                        }
                    >
                        <InAppLoginPanel
                            providerId={CLAUDE_PROVIDER.id}
                            providerLabel={CLAUDE_PROVIDER.displayName}
                            authUrl={authUrl()}
                            phase={phase()}
                            onCancel={onCancel}
                            onUseTerminal={() => {
                                terminalRequested = true;
                                setPhase("fallback");
                            }}
                        />
                    </Show>
                </Show>
            </div>
        </div>
    );
}

ClaudeLoginPanel.displayName = "ClaudeLoginPanel";
