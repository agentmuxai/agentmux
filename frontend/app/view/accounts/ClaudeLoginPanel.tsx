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

import { createSignal, Show, type JSX } from "solid-js";
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
}): JSX.Element {
    const [phase, setPhase] = createSignal<InAppLoginPhase>("starting");
    const [authUrl, setAuthUrl] = createSignal<string | undefined>(undefined);
    const [error, setError] = createSignal<string | null>(null);
    const [done, setDone] = createSignal(false);
    let cancelled = false;
    let terminalRequested = false;
    let inFlight = false;

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
                    void refreshAccountCache();
                    setDone(true);
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
