// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Claude sign-in's session: finding the CLI, running the login (in-app, then the terminal if asked), and cancelling it. */

import { translateError } from "@/app/errors/translate";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ensureProviderAuthDir } from "@/app/view/agent/agent-launch-env";
import { loginBackend } from "@/app/view/agent/flows/login-backend";
import { withLoginWindow } from "@/app/view/agent/flows/login-window";
import { runProviderLogin, type ProviderLoginOutcome } from "@/app/view/agent/flows/run-provider-login";
import { applyOutcome, confirmLink, needsLinkCheck } from "./claude-login-finish";
import { CLAUDE_PROVIDER } from "./claude-login-model";
import type { LoginState } from "./claude-login-state";

/** The CLI's path, or null after saying why it couldn't be found. */
async function resolveCliPath(s: LoginState): Promise<string | null> {
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
        return r.cli_path;
    } catch (e) {
        const t = translateError(e);
        s.setError(`${t.title}: ${t.message}${t.retry ? ` — ${t.retry}` : ""}`);
        return null;
    }
}

/** The login's environment, or null after saying why the auth directory couldn't be made. */
async function buildAuthEnv(s: LoginState): Promise<Record<string, string> | null> {
    const authEnv: Record<string, string> = {};
    if (!CLAUDE_PROVIDER.authConfigDirEnvVar) return authEnv;
    try {
        authEnv[CLAUDE_PROVIDER.authConfigDirEnvVar] = await ensureProviderAuthDir(CLAUDE_PROVIDER.id);
    } catch (e) {
        s.setError((e as Error)?.message ?? String(e));
        return null;
    }
    return authEnv;
}

export function createLoginSession(s: LoginState) {
    const props = s.props;

    const runAttempt = async (cliPath: string, authEnv: Record<string, string>, skipTier1: boolean) =>
        runProviderLogin({
            provider: CLAUDE_PROVIDER,
            cliPath,
            authEnv,
            // Prefer the account THIS panel already minted/persisted
            // (registeredAccountId, set by onAccountRegistered below) over the
            // original prop. Without this, clicking Retry after a link-
            // verification failure (the account persisted fine, but confirming
            // its link to this agent failed) re-ran with the stale
            // props.existingAccountId — undefined for Connectors' bare Connect —
            // minting a SECOND fresh account and orphaning the first one, which
            // is already a real, credentialed Claude account. Falls back to
            // props.existingAccountId when nothing has been registered yet
            // (the ordinary first-attempt / non-persist-failure retries) (#2414).
            existingAccountId: s.registeredAccountId ?? props.existingAccountId,
            linkTarget: props.linkTarget,
            // The in-app tier 1 runs first; "Use terminal instead" re-runs
            // with it skipped.
            skipTier1,
            awaitTier1Completion: true,
            setAuthUrl: (url) => {
                s.setAuthUrl(url ?? undefined);
                if (url) s.setPhase("waiting-authorize");
            },
            log: (_cat, msg) => console.log(`[claude-login] ${msg}`),
            isCancelled: () => s.cancelled || s.terminalRequested,
            onAccountRegistered: (accountId) => {
                s.registeredAccountId = accountId;
            },
            onTierChange: (event) => {
                if (event.tier === "inapp-waiting") s.setPhase("waiting-authorize");
                else if (event.tier === "fallback") s.setPhase("fallback");
                else s.setPhase("terminal-polling");
            },
        });

    const startFlow = async () => {
        if (s.inFlight()) return;
        s.setInFlight(true);
        s.setError(null);
        s.setPhase("starting");
        try {
            const cliPath = await resolveCliPath(s);
            if (cliPath == null) return;
            const authEnv = await buildAuthEnv(s);
            if (authEnv == null) return;
            if (s.cancelled) return;

            let outcome: ProviderLoginOutcome = await runAttempt(cliPath, authEnv, false);
            if (outcome === "inapp-timeout" && s.terminalRequested && !s.cancelled) {
                // "Use terminal instead" was asked for during the in-app tier:
                // release the request and re-run with the terminal.
                s.terminalRequested = false;
                s.setPhase("fallback");
                outcome = await runAttempt(cliPath, authEnv, true);
            }
            if (s.cancelled) return;
            const linkConfirmed = needsLinkCheck(s, outcome) ? await confirmLink(s) : true;
            applyOutcome(s, outcome, linkConfirmed);
        } catch (e) {
            // runAttempt (runProviderLogin) can REJECT outright — e.g. the PTY
            // child fails to spawn, or an IPC call errors — not just resolve to
            // a failure outcome string. Without this catch the rejection
            // escaped `void start()`'s fire-and-forget call as an unhandled
            // rejection, leaving the panel stuck on its starting phase forever
            // with no error shown and no Retry button (#2410).
            loginBackend().cancel().catch(() => {});
            const t = translateError(e);
            s.setError(`${t.title}: ${t.message}${t.retry ? ` — ${t.retry}` : ""}`);
        } finally {
            s.setInFlight(false);
        }
    };

    // The login's window opens inside the click (its URL comes later), and is
    // closed on every way out of the flow if no URL used it (login-window.ts).
    const start = () => (s.inFlight() ? Promise.resolve() : withLoginWindow(CLAUDE_PROVIDER, startFlow));

    const onCancel = () => {
        s.cancelled = true;
        loginBackend().cancel().catch(() => {});
        props.onClose();
    };

    const useTerminal = () => {
        s.terminalRequested = true;
        s.setPhase("fallback");
    };

    return { start, onCancel, useTerminal };
}

export type LoginSession = ReturnType<typeof createLoginSession>;
