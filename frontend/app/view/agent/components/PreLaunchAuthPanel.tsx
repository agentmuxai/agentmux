// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PreLaunchAuthPanel — the Connect-with-OAuth UI block that sits
 * inline in `AgentLaunchModal` before the Launch button. The
 * `AuthFlowController` is owned by the parent (`AgentLaunchModal`)
 * and passed in as a prop so the controller's lifetime spans the
 * whole modal, not just this panel's conditional `<Show>` mount.
 *
 * Spec: `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` §3 + §8.
 *
 * Renders four mutually-exclusive panels off `controller.state().kind`:
 *   - `unauthenticated` / `expired` — Connect CTA (OAuth or API-key)
 *   - `waiting` — "Waiting for OAuth…" with URL fallback + Cancel
 *   - `ready` — green check banner ("Connected as <email>")
 *   - `failed` — red error banner + retry CTA
 *
 * The parent reads `props.controller.state()` directly to gate the
 * Launch button on `state().kind === "ready"`.
 */

import { Button } from "@/element/button";
import { translateError } from "@/app/errors/translate";
import { getApi } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import {
    createEffect,
    createSignal,
    Match,
    Show,
    Switch,
    untrack,
    type Accessor,
    type JSX,
} from "solid-js";

import {
    AuthFlowController,
    type AuthState,
    type SelectionOutcome,
} from "../auth";
import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { InAppLoginPanel, type InAppLoginPhase } from "./InAppLoginPanel";
import { runProviderLogin } from "../flows/run-provider-login";
import type { ProviderDefinition } from "../providers";
import "./PreLaunchAuthPanel.scss";

export interface PreLaunchAuthPanelProps {
    /** The provider to authenticate. */
    provider: ProviderDefinition | undefined;
    /** Currently-selected account id, or "" if the user hasn't picked
     *  one yet. Issue #1624 PR-C Part B — was `identityId` (a bundle
     *  id); OAuth Connect no longer requires a pre-selected account,
     *  so the empty case is just "nothing chosen yet," not a gate. */
    accountId: Accessor<string>;
    /** True when the selected account actually supplies credentials
     *  for the agent's provider. Replaces `hasMatchingBinding`. */
    accountSuppliesProvider: Accessor<boolean>;
    /** The selected account's own `status` field (`"valid" |
     *  "expired" | "needs_reauth" | "unknown"` for oauth-class
     *  accounts), or `null` when no account is selected. Replaces
     *  `bindingStatus` — with a direct account selection instead of a
     *  bundle→binding→account join, this is just the account's own
     *  status. When `"expired"`/`"needs_reauth"`, the Connect CTA's
     *  wording shifts to "credentials need reconnecting". */
    accountStatus?: Accessor<string | null>;
    /** Auth flow controller — owned by the Launch modal so its
     *  lifetime spans the whole modal, not just the panel's
     *  conditional mount. Lifting fixed the "memory change forgot
     *  login" bug where a brief re-render unmounted this panel and
     *  destroyed an internally-constructed controller along with it.
     *  See docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md. */
    controller: AuthFlowController;
    /** Called when a new account is created via OAuth — parent should
     *  refresh the account list and select it. Replaces
     *  `onBundleCreated`. */
    onAccountCreated: (accountId: string) => void;
    /** Called when the user clicks "+ Add account" for a manual
     *  (API-key) account — separate from OAuth Connect, which no
     *  longer needs any pre-step (issue #1624 PR-C Part B removed the
     *  "+ New identity bundle" interposition; OAuth starts directly). */
    onRequestAddAccount?: () => void;
    /** Inline-disabled flag mirroring the modal's submitting state.
     *  Prevents starting a Connect mid-launch. */
    disabled?: boolean;
}

/** Sentinel session id for logins driven by `runProviderLogin` (the
 *  `requiresLoginTty` branch of `startConnect`), which bypass the
 *  `auth.start`/`auth.poll` RPC session machinery entirely. Doubles as the
 *  render switch: a `waiting` state carrying this id means "in-app login
 *  session" (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.1) and renders
 *  `InAppLoginPanel` (URL + paste-code → `setProviderAuth`) instead of the
 *  generic `WaitingPanel` (whose paste row submits a redirect URL to
 *  `auth.submitcallback` — a backend session this flow doesn't have). */
const PROVIDER_LOGIN_SESSION_ID = "provider-login";

/** UI hooks `startConnect` (a module-level function) uses to drive the
 *  panel-owned in-app-login signals: the live phase line, and the
 *  "Use terminal instead" request flag. */
interface InAppLoginUi {
    setPhase: (phase: InAppLoginPhase) => void;
    terminalRequested: () => boolean;
    setTerminalRequested: (v: boolean) => void;
}

export const PreLaunchAuthPanel = (props: PreLaunchAuthPanelProps): JSX.Element => {
    // Controller is owned by the parent (AgentLaunchModal); panel
    // mount/unmount doesn't construct or dispose it. The parent
    // reads `controller.state()` directly for `authStateKind()`
    // — no more `onStateChange` callback wiring.
    const controller = props.controller;

    // In-app login session UI state (see InAppLoginPhase / InAppLoginUi).
    const [inAppPhase, setInAppPhase] = createSignal<InAppLoginPhase>("starting");
    const [terminalRequested, setTerminalRequested] = createSignal(false);
    const inAppUi: InAppLoginUi = {
        setPhase: setInAppPhase,
        terminalRequested,
        setTerminalRequested,
    };

    // Auto-open the OAuth URL in the user's default browser as soon as
    // the auth controller surfaces it. Same effect as the legacy
    // launch-flow.ts inline path. The URL also stays visible in the
    // WaitingPanel below with a Copy button — that's the manual
    // fallback when the OS doesn't route the open (browser closed,
    // protocol handler missing, etc.). Fires once per URL (the guard
    // tracks the last URL we opened so re-renders don't re-fire).
    //
    // Skipped for the in-app login session (PROVIDER_LOGIN_SESSION_ID):
    // there, `forceProviderLogin` already opened the URL itself
    // (openOAuthBrowserPane — system browser with an in-app-pane fallback)
    // the moment it was captured; opening it a second time here would spawn
    // a duplicate tab/pane for every Claude connect.
    let lastOpenedUrl: string | null = null;
    createEffect(() => {
        const s = controller.state();
        const url = s.authUrl;
        if (s.sessionId === PROVIDER_LOGIN_SESSION_ID) return;
        if (url && url !== lastOpenedUrl) {
            lastOpenedUrl = url;
            console.log(`[auth-diag] opening auth URL in browser (host=${(() => { try { return new URL(url).host; } catch { return "?"; } })()})`);
            try {
                getApi().openExternal(url);
            } catch (e) {
                console.warn(`[auth-diag] openExternal failed: ${(e as Error)?.message ?? String(e)}`);
            }
        }
    });

    // Forward `succeeded` to account-creation callback so the modal
    // selects the newly-persisted account. Replaces the old bundle-id
    // placeholder filter (`pending-bundle-for-<sid>`) — that synthetic
    // only ever applied to the legacy bundle path; a direct-account
    // session's `bundleId` (see auth-state.ts's foldPolled comment for
    // why the field itself isn't renamed) is either empty (failure, no
    // persist) or a real account id, never a placeholder.
    createEffect(() => {
        const s = controller.state();
        if (s.kind === "ready" && s.bundleId) {
            props.onAccountCreated(s.bundleId);
        }
    });

    // When the account dropdown changes, drive the controller's
    // `selected` action. The outcome is computed below.
    //
    // `untrack` around the controller call: `controller.selected()`
    // calls `dispatch()` which reads `_state` internally. Without
    // untrack, Solid would add `_state` as a tracked dep of THIS
    // effect — every subsequent dispatch (from connect/poll/etc.)
    // would re-fire `selected`, which calls `stopPolling()` and
    // dispatches `Selected`, wiping any in-flight session. That's
    // exactly the bug the diagnostic logs caught.
    createEffect(() => {
        const id = props.accountId();
        const prov = props.provider;
        const suppliesProvider = props.accountSuppliesProvider();
        if (!prov) return;
        // "" id (no account selected) is a genuinely fresh connect —
        // the reducer's `Selected` command still takes a "bundleId"-
        // named field (unrenamed, see auth-state.ts), but the value
        // here is an account id.
        untrack(() => controller.selected(prov.id, id, outcomeFor(id, suppliesProvider)));
    });

    // Connect / Retry click handler. Issue #1624 PR-C Part B: OAuth no
    // longer requires a pre-selected account — the backend mints one
    // directly (`direct_account: true` in auth-flow-controller.ts's
    // `connect()`). This used to gate on `identityId() === ""` and
    // route through a "+ New identity bundle" interposition
    // (docs/specs/archive/SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md §4.5's "OAuth never
    // starts without a bundle id" invariant) — that invariant no
    // longer applies; the per-account isolation dir is resolved
    // server-side regardless of whether an account was pre-selected.
    const handleConnect = (): void => {
        const prov = props.provider;
        if (!prov) return;
        void startConnect(controller, prov, props.accountId(), inAppUi);
    };

    // "Use my existing login" REMOVED 2026-08-31 — it copied the operator's
    // personal ~/.claude credential into a minted account dir, which defeated
    // per-channel isolation (an agent could reach Claude in a channel that was
    // never logged into). See
    // docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md #3.
    // Connect (the in-app OAuth session) is the single pre-launch path now.

    // No onCleanup here — controller lifetime is owned by the parent
    // (AgentLaunchModal). Disposing on panel unmount would destroy
    // in-flight auth state any time the parent's `<Show when={authRequired()}>`
    // briefly flips false (the root cause of the "memory change forgot
    // login" bug fixed in this PR).

    // The Connect CTA has two triggers and one arm:
    //   - the controller has no usable credentials (`unauthenticated`) or
    //     the probe said the session is gone (`expired`), or
    //   - the controller is `ready` — outcomeFor() only asks "does the
    //     account supply this provider?" — but the backend's expiry probe
    //     flagged the account itself `needs_reauth` / `expired`. Without
    //     this the panel would render <ReadyBanner /> and ConnectCta's
    //     reconnect wording would never be seen; the user would have no
    //     signal their credentials need refreshing. Clicking Connect
    //     re-runs OAuth into the SAME account's isolation dir
    //     (existingAccountId in auth-flow-controller.ts's connect()),
    //     refreshing the token in place. codex P1 on #982.
    // The arm is listed first so the stale-ready case wins over the
    // generic ready arm; `unauthenticated` / `expired` match no other arm,
    // so folding them in here changes which arm renders for no state.
    const showConnectCta = () => {
        const kind = controller.state().kind;
        if (kind === "unauthenticated" || kind === "expired") return true;
        const status = props.accountStatus?.();
        return kind === "ready" && (status === "needs_reauth" || status === "expired");
    };

    return (
        <div class="pre-launch-auth-panel">
            <Switch>
                <Match when={showConnectCta()}>
                    <ConnectCta
                        provider={props.provider}
                        state={controller.state()}
                        accountStatus={props.accountStatus?.() ?? null}
                        hasAccount={props.accountSuppliesProvider()}
                        onConnect={() => handleConnect()}
                        disabled={props.disabled ?? false}
                    />
                </Match>
                <Match when={controller.state().kind === "ready"}>
                    <ReadyBanner />
                </Match>
                {/* In-app login session (PROVIDER_LOGIN_SESSION_ID sentinel —
                    see its doc comment): URL + paste-code UI wired to the
                    login child's stdin via setProviderAuth, with a live phase
                    line and an explicit (never auto-launched) terminal
                    fallback. Must match BEFORE the generic waiting arm. */}
                <Match
                    when={
                        controller.state().kind === "waiting" &&
                        controller.state().sessionId === PROVIDER_LOGIN_SESSION_ID
                    }
                >
                    <InAppLoginPanel
                        providerId={props.provider?.id ?? ""}
                        providerLabel={props.provider?.displayName ?? "this provider"}
                        authUrl={controller.state().authUrl}
                        phase={inAppPhase()}
                        onCancel={() => void controller.cancel()}
                        onUseTerminal={() => {
                            setTerminalRequested(true);
                            setInAppPhase("fallback");
                        }}
                    />
                </Match>
                <Match when={controller.state().kind === "waiting"}>
                    <WaitingPanel
                        state={controller.state()}
                        onCancel={() => void controller.cancel()}
                        onSubmitCallback={(url) => void controller.submitCallback(url)}
                    />
                </Match>
                <Match
                    when={
                        controller.state().kind === "authenticated" ||
                        controller.state().kind === "saving"
                    }
                >
                    {/* Codex P1 on #853 round 7: minimal stub so the
                        panel doesn't go blank when the reducer enters
                        the new `authenticated`/`saving` kinds. The
                        rich SaveBundle UI lands in PR C-4 along with
                        the backend `auth.savebundle` RPC; this stub
                        keeps the user informed in the interim.
                        Reagent P2 on #853 round 11: surface the Cancel
                        action so users aren't forced to close the modal. */}
                    <AuthenticatedStub
                        state={controller.state()}
                        onCancel={() => void controller.cancel()}
                    />
                </Match>
                <Match when={controller.state().kind === "failed"}>
                    <FailedBanner
                        state={controller.state()}
                        onRetry={() => handleConnect()}
                        canRetry={!props.disabled}
                    />
                </Match>
            </Switch>
        </div>
    );
};

/** Compute the SelectionOutcome from the account id and whether it
 *  supplies the agent's provider. No account selected, or a selected
 *  account for a different provider, → `needs-account`; a matching
 *  account → `ready`. */
function outcomeFor(
    accountId: string,
    accountSuppliesProvider: boolean,
): SelectionOutcome {
    if (!accountId || !accountSuppliesProvider) return "needs-account";
    return "ready";
}

async function startConnect(
    controller: AuthFlowController,
    provider: ProviderDefinition | undefined,
    existingAccountId: string,
    ui: InAppLoginUi,
): Promise<void> {
    console.log(`[auth-diag] startConnect entry: provider=${provider?.id ?? "(undefined)"} requiresLoginTty=${provider?.requiresLoginTty}`);
    if (!provider) {
        console.warn("[auth-diag] startConnect: provider undefined, bailing");
        return;
    }
    // Resolve the CLI path via the same RPC `launch-flow.ts` uses.
    // The backend handles "not installed → npm install" so the
    // call returns a valid path or an error.
    let cliPath: string;
    try {
        const r = await RpcApi.ResolveCliCommand(
            TabRpcClient,
            {
                provider_id: provider.id,
                cli_command: provider.cliCommand,
                npm_package: provider.npmPackage,
                pinned_version: provider.pinnedVersion,
                windows_install_command: provider.windowsInstallCommand,
                unix_install_command: provider.unixInstallCommand,
            },
            { timeout: 120000 },
        );
        cliPath = r.cli_path;
        console.log(`[auth-diag] ResolveCli ok: cliPath=${cliPath}`);
    } catch (e) {
        console.error(`[auth-diag] ResolveCli FAILED: ${(e as Error)?.message ?? String(e)}`);
        // Surface the real ResolveCli error in the FailedBanner —
        // reagent P1 on #847: previously this discarded `e` and
        // called connect with an empty cliPath, producing a
        // misleading backend "CLI not found at ''" message.
        //
        // Route through translateError so typed wire-format errors
        // render as readable text in the FailedBanner instead of
        // raw JSON. Legacy free-text errors pass through unchanged.
        const t = translateError(e);
        controller.failConnect(new Error(`${t.title}: ${t.message}${t.retry ? ` — ${t.retry}` : ""}`));
        return;
    }
    // Codex P1 on #847: pass the full provider-isolated auth env. Setting
    // `authConfigDirEnvVar` (e.g. CLAUDE_CONFIG_DIR) to the per-provider
    // isolated dir matters because the CLI writes creds to whatever this
    // env var points at; without it, the auth.start subprocess writes
    // into the user's global ~/.claude, not the version-isolated dir
    // the subsequent agent launch reads from.
    const authEnv: Record<string, string> = {};
    if (provider.authConfigDirEnvVar) {
        try {
            const authDir = await getApi().ensureAuthDir(provider.id);
            authEnv[provider.authConfigDirEnvVar] = authDir;
            console.log(`[auth-diag] ensureAuthDir ok: ${provider.authConfigDirEnvVar}=${authDir}`);
        } catch (e) {
            console.error(`[auth-diag] ensureAuthDir FAILED: ${(e as Error)?.message ?? String(e)}`);
            controller.failConnect(e);
            return;
        }
    }
    if (provider.authExtraEnv) {
        Object.assign(authEnv, provider.authExtraEnv);
    }
    // Codex P2 on #854 round 4: bail before connect() if the modal
    // closed mid-prep. The controller itself also gates on `closed`,
    // but skipping the call avoids the extra RPC bookkeeping.
    if (controller.state().closed) {
        console.warn("[auth-diag] startConnect: controller closed mid-prep, bailing");
        return;
    }

    // requiresLoginTty providers (Claude, OpenClaw) route through
    // `runProviderLogin` instead of `controller.connect()`'s `auth.start`/
    // `auth.poll` RPC path — that path's spawn (`spawn_auth_cli`/
    // `spawn_auth_cli_pty`) can't drive these CLIs' interactive logins and
    // was a structural dead end for both providers (see PR #2262's history).
    //
    // Since SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md this branch IS the
    // in-app login session (spec §3.1, surface 1): tier 1 runs (no more
    // hardcoded `skipTier1: true` — the 2026-08-03 probes confirmed Claude
    // 2.1.198+ prints the authorize URL under our PTY spawn and accepts a
    // pasted code on stdin), `awaitTier1Completion` keeps the session inside
    // `runProviderLogin` until the login child exits and the credential
    // lands in the minted isolated dir, and `InAppLoginPanel` (rendered via
    // the PROVIDER_LOGIN_SESSION_ID sentinel) shows URL + paste box + live
    // phase + Cancel. The terminal tier is never auto-launched from here on
    // the happy path — it remains reachable two ways: the behavior-gate
    // (an older CLI that prints no URL within the capture window falls
    // through to tiers 2/3 exactly as before), and the user's explicit
    // "Use terminal instead" click (ui.terminalRequested), which releases
    // the in-app wait and re-runs the login with tier 1 skipped.
    if (provider.requiresLoginTty) {
        // reagent P1 on #2262: the Reconnect arm (stale needs_reauth/expired
        // account) leaves the reducer in `ready` for this whole call —
        // `ConnectClicked` below is silently dropped by connect()'s own
        // unauthenticated/expired/failed guard, so `state().kind` never
        // moves to `waiting` and can't distinguish "first click" from "a
        // second click while the first login is still running." Without an
        // explicit guard, a double-click spawned two concurrent
        // terminal-login processes against the same account dir.
        if (!controller.beginTtyLogin()) {
            console.warn("[auth-diag] startConnect: requiresLoginTty login already in flight, ignoring click");
            return;
        }
        // Declared outside the try so the catch clause below can also see
        // it (a rejection can happen before or after it's assigned).
        let actionToken: ReturnType<typeof controller.currentActionToken> | undefined;
        try {
            controller.dispatch({ type: "ConnectClicked" });
            // Mount the in-app login panel immediately, before any URL is
            // captured — the sentinel session id is the render switch (see
            // PROVIDER_LOGIN_SESSION_ID's doc), and an empty authUrl just
            // renders the live phase line ("requesting a sign-in link…")
            // with Cancel / "Use terminal instead" available from second 0.
            controller.dispatch({ type: "SessionStarted", sessionId: PROVIDER_LOGIN_SESSION_ID });
            ui.setPhase("starting");
            ui.setTerminalRequested(false);
            // reagent P1 on #2262: this branch isn't gated through
            // connect()'s own actionToken staleness check at all (it
            // bypasses connect() entirely for requiresLoginTty providers).
            // The account/provider dropdown isn't disabled while a tty
            // login is in flight (only `disabled`/`submitting()` gates it),
            // so a `Selected` dispatch mid-flight resets state to the new
            // selection — but without this snapshot, the ABANDONED login's
            // eventual markSeeded/failConnect would still land on top of
            // it once runProviderLogin resolves, clobbering the new
            // selection's state with the old attempt's outcome.
            actionToken = controller.currentActionToken();
            let registeredAccountId = "";
            const runAttempt = (skipTier1: boolean) =>
                runProviderLogin({
                    provider,
                    cliPath,
                    authEnv,
                    existingAccountId: existingAccountId || undefined,
                    // Behavior-gate only (no catalog provider sets the flag
                    // since Claude's was dropped — see catalog.ts): the
                    // in-app tier 1 runs first; a CLI that prints no URL
                    // within the capture window falls through to tiers 2/3
                    // inside runProviderLogin exactly as before. The second
                    // attempt (the user's explicit "Use terminal instead")
                    // passes true to go straight there.
                    skipTier1: skipTier1 || provider.headlessLoginUrlUnsupported === true,
                    // The whole in-app session lives inside runProviderLogin:
                    // it polls for child-exit + credential-landed, persists
                    // and links the account, and resolves with
                    // "inapp-success"/"inapp-timeout" — no hand-rolled
                    // "opened" completion poll in this caller.
                    awaitTier1Completion: true,
                    onAccountRegistered: (accountId) => {
                        registeredAccountId = accountId;
                    },
                    // Captured URL → InAppLoginPanel (the reducer accepts a
                    // repeat SessionStarted while `waiting`, updating
                    // authUrl in place). forceProviderLogin opens the
                    // browser itself; the panel's own auto-open effect is
                    // suppressed for this sentinel to avoid a double open.
                    setAuthUrl: (url) => {
                        controller.dispatch({
                            type: "SessionStarted",
                            sessionId: PROVIDER_LOGIN_SESSION_ID,
                            authUrl: url ?? undefined,
                        });
                        if (url) ui.setPhase("waiting-authorize");
                    },
                    log: (_cat, msg) => console.log(`[auth-diag] ${msg}`),
                    // Releases the in-app/tier-3 waits when the user clicks
                    // Cancel (controller.wasCancelled — reagent P2 on #2262:
                    // state can leave `waiting` for non-cancel reasons, so
                    // the explicit flag is the only trustworthy signal), OR
                    // clicks "Use terminal instead" (terminalRequested —
                    // same release mechanism, different follow-up below), OR
                    // the action token itself went stale (changed account/
                    // provider selection, or the modal closed — the SAME
                    // condition the post-hoc discard at isStaleAction(...)
                    // below checks). Without isStaleAction here too, a
                    // selection change didn't stop the poll — with
                    // awaitTier1Completion, runProviderLogin persists+links
                    // the account INTERNALLY before this call even returns,
                    // so the post-hoc discard below only hid the stale
                    // outcome from the reducer; it couldn't undo a link/
                    // persist that had already happened for a selection the
                    // user had already moved away from (reagent P2 on
                    // PR #2410). runProviderLogin reaps the login CLI child
                    // itself on its way out of the awaited in-app wait.
                    isCancelled: () =>
                        controller.wasCancelled() ||
                        ui.terminalRequested() ||
                        controller.isStaleAction(actionToken),
                    // Keep the live phase line honest across the real
                    // transitions (reagent P1 on PR #2300's onTierChange
                    // rationale, applied to this panel).
                    onTierChange: (event) => {
                        if (event.tier === "inapp-waiting") {
                            ui.setPhase("waiting-authorize");
                        } else if (event.tier === "fallback") {
                            ui.setPhase("fallback");
                        } else {
                            ui.setPhase("terminal-polling");
                        }
                    },
                });
            let outcome = await runAttempt(false);
            if (controller.isStaleAction(actionToken)) {
                // The user moved on (changed selection, cancelled, or the
                // modal closed) while this login was in flight — its
                // outcome belongs to an abandoned attempt and must not
                // touch whatever the controller is doing now.
                console.warn(`[auth-diag] startConnect: requiresLoginTty login outcome (${outcome}) is stale, discarding`);
                return;
            }
            if (outcome === "inapp-timeout" && ui.terminalRequested() && !controller.wasCancelled()) {
                // "Use terminal instead": the flag released the in-app wait
                // (isCancelled above), which surfaces as "inapp-timeout".
                // Reset it BEFORE the terminal attempt — it's part of that
                // same isCancelled closure, and leaving it set would abort
                // the terminal tier's own completion poll on its first tick.
                ui.setTerminalRequested(false);
                ui.setPhase("fallback");
                outcome = await runAttempt(true);
                if (controller.isStaleAction(actionToken)) {
                    console.warn(`[auth-diag] startConnect: terminal-fallback login outcome (${outcome}) is stale, discarding`);
                    return;
                }
            }
            switch (outcome) {
                case "inapp-success":
                case "terminal-success":
                    if (registeredAccountId) {
                        controller.markSeeded(registeredAccountId);
                    } else {
                        // The credential landed but the Armory account row
                        // couldn't be persisted — don't show fake-ready state
                        // (the resolver's spawn gate requires a real bound
                        // account, so a fake-ready panel would let Launch
                        // enable and then have the agent immediately fail).
                        controller.failConnect(
                            new Error("Login completed but the account couldn't be registered. Try again."),
                        );
                    }
                    break;
                case "inapp-timeout":
                case "terminal-timeout":
                    // reagent P1 on #2262: an explicit user Cancel already
                    // reset state to unauthenticated via CancelClicked —
                    // showing a "wasn't completed within 5 minutes" failure
                    // banner on top of that would be actively misleading
                    // about what actually happened (mirrors relogin()'s
                    // same wasCancelled() guard on its own "opened" timeout
                    // message).
                    if (!controller.wasCancelled()) {
                        controller.failConnect(
                            new Error("Login wasn't completed within 5 minutes. Click Connect to try again."),
                        );
                    }
                    break;
                case "terminal-unavailable":
                    controller.failConnect(
                        new Error(`Couldn't open a terminal for ${provider.displayName} login on this platform.`),
                    );
                    break;
                case "opened":
                    // Only reachable when the isolated account-dir mint failed
                    // (runProviderLogin downgrades an awaited tier 1 to the
                    // legacy "opened" contract when there's no dir to poll or
                    // persist against). No fake-ready: without a minted
                    // account the resolver's spawn gate would block the agent
                    // anyway, so surface it as the failure it is.
                    controller.failConnect(
                        new Error("Login started, but AgentMux couldn't prepare an isolated account for it. Try again."),
                    );
                    break;
            }
        } catch (e) {
            // reagent P2 on PR #2410: runAttempt (runProviderLogin ->
            // forceProviderLogin -> getApi().runCliLogin) can REJECT outright
            // — e.g. the PTY child fails to spawn, or the IPC call itself
            // errors — not just resolve to a failure outcome string. This
            // try previously had no catch at all, so a rejection here
            // escaped past `void startConnect(...)`'s fire-and-forget call
            // as an unhandled rejection, leaving the reducer stuck in the
            // sentinel `waiting` state (the panel shows "requesting a
            // sign-in link…" forever) with no failed banner and no way to
            // retry short of closing the modal. Reap any partial child and
            // report it the same way every other failure path here does.
            getApi().cancelCliLogin().catch(() => {});
            if (actionToken === undefined || !controller.isStaleAction(actionToken)) {
                controller.failConnect(e);
            }
        } finally {
            controller.endTtyLogin();
        }
        return;
    }

    console.log(`[auth-diag] calling controller.connect(); kind=${controller.state().kind}`);
    await controller.connect({
        cliPath,
        authLoginArgs: provider.authLoginCommand,
        authCheckArgs: provider.authCheckCommand,
        authEnv,
        requiresTty: provider.requiresLoginTty ?? false,
    });
    console.log(`[auth-diag] controller.connect returned; kind=${controller.state().kind}`);
}

// ── Sub-panels ─────────────────────────────────────────────────────

const ConnectCta = (p: {
    provider: ProviderDefinition | undefined;
    state: AuthState;
    /** The selected account's own `status`, or `null` when no account
     *  is selected. Drives the spec §4.4 status-aware wording when
     *  the account's tokens are `expired` / `needs_reauth`. */
    accountStatus: string | null;
    /** True when `accountSuppliesProvider(state, provider)` is true —
     *  gates the reconnect-wording branch so it only triggers when
     *  there IS an account to refresh (the no-account case keeps the
     *  generic Connect CTA wording the user already knows). */
    hasAccount: boolean;
    onConnect: () => void;
    disabled: boolean;
}): JSX.Element => {
    const catalog = () =>
        p.provider ? getCliCatalogEntry(p.provider.id) : undefined;
    const providerLabel = () => p.provider?.displayName ?? "this provider";
    const isExpired = () => p.state.kind === "expired";
    // Spec §4.4 — when the user has an account for this provider but
    // its status flags a reconnect (expired / needs_reauth), shift to
    // the "credentials need reconnecting" wording. Same Connect
    // button, same OAuth flow underneath; just a clearer nudge so the
    // user understands they're refreshing a known login rather than
    // starting fresh.
    const needsReconnect = () =>
        p.hasAccount &&
        (p.accountStatus === "needs_reauth" || p.accountStatus === "expired");
    return (
        <div class="pre-launch-auth-panel-cta">
            <div class="pre-launch-auth-panel-warning">
                {needsReconnect()
                    ? `⚠ Your ${providerLabel()} credentials need reconnecting.`
                    : isExpired()
                        ? `⚠ Your ${providerLabel()} session is expired. Re-authenticate before launching.`
                        : `⚠ ${providerLabel()} requires a login before launch.`}
            </div>
            <Button
                onClick={() => p.onConnect()}
                disabled={p.disabled}
                className="pre-launch-auth-panel-connect green solid"
            >
                <span class="pre-launch-auth-panel-connect-icon" aria-hidden="true">
                    {catalog()?.icon ?? "🔐"}
                </span>
                <span class="pre-launch-auth-panel-connect-label">
                    {needsReconnect()
                        ? `Reconnect ${providerLabel()}`
                        : isExpired()
                            ? "Re-authenticate"
                            : `Connect to ${providerLabel()}`}
                </span>
            </Button>
            <div class="pre-launch-auth-panel-hint">
                {needsReconnect()
                    ? `Re-runs OAuth into your existing account. Launch stays available — your CLI will refresh on first call.`
                    : `Opens browser → ${providerLabel()} login → returns to AgentMux.
                    Tokens get saved as a new account so the next agent doesn't
                    have to re-authenticate.`}
            </div>
        </div>
    );
};

const WaitingPanel = (p: {
    state: AuthState;
    onCancel: () => void;
    onSubmitCallback: (url: string) => void;
}): JSX.Element => {
    const [pasted, setPasted] = createSignal("");
    return (
        <div class="pre-launch-auth-panel-waiting">
            <div class="pre-launch-auth-panel-waiting-title">
                🔐 Waiting for OAuth…
            </div>
            <ol class="pre-launch-auth-panel-waiting-steps">
                <li>Authorize AgentMux in your browser tab.</li>
                <li>We'll detect the redirect and continue.</li>
            </ol>
            <Show when={p.state.authUrl}>
                <div class="pre-launch-auth-panel-url-label">
                    Auth URL (browser should have opened — if not, copy this):
                </div>
                <div class="pre-launch-auth-panel-url-row">
                    <code
                        class="pre-launch-auth-panel-url-text"
                        title={p.state.authUrl}
                    >
                        {p.state.authUrl}
                    </code>
                    <Button
                        className="grey solid"
                        onClick={() => {
                            // Route through the CEF clipboard wrapper —
                            // navigator.clipboard.* is fragile under CEF's
                            // permission policy. See
                            // SPEC_UNIFIED_CLIPBOARD_2026_05_18.md §3.3.
                            void clipboardWriteText(p.state.authUrl).catch((err) =>
                                console.log("clipboard write failed", err),
                            );
                        }}
                    >
                        Copy
                    </Button>
                </div>
                <div class="pre-launch-auth-panel-callback-row">
                    <input
                        class="pre-launch-auth-panel-url-input"
                        placeholder="Paste the redirect URL here if the browser didn't return automatically"
                        value={pasted()}
                        onInput={(e) => setPasted(e.currentTarget.value)}
                    />
                    <Button
                        className="grey solid"
                        onClick={() => {
                            const url = pasted().trim();
                            if (url) p.onSubmitCallback(url);
                        }}
                        disabled={pasted().trim().length === 0}
                    >
                        Submit
                    </Button>
                </div>
            </Show>
            <Show when={p.state.deviceCode}>
                <div class="pre-launch-auth-panel-device-code">
                    <div class="pre-launch-auth-panel-device-code-label">
                        Enter this code at{" "}
                        <a
                            href={p.state.deviceCode!.verificationUrl}
                            target="_blank"
                            rel="noopener noreferrer"
                        >
                            {p.state.deviceCode!.verificationUrl}
                        </a>
                    </div>
                    <div class="pre-launch-auth-panel-device-code-value">
                        {p.state.deviceCode!.code}
                    </div>
                </div>
            </Show>
            <Button onClick={() => p.onCancel()}>Cancel login</Button>
        </div>
    );
};

const ReadyBanner = (): JSX.Element => (
    <div class="pre-launch-auth-panel-ready">
        ✓ Connected. Ready to launch.
    </div>
);

const AuthenticatedStub = (p: {
    state: AuthState;
    onCancel: () => void;
}): JSX.Element => (
    <div class="pre-launch-auth-panel-ready">
        <div>
            ✓ Authenticated{p.state.email ? ` as ${p.state.email}` : ""}.
            {" "}Awaiting bundle save…
        </div>
        <Button onClick={() => p.onCancel()} className="pre-launch-auth-panel-cancel">
            Cancel
        </Button>
    </div>
);

const FailedBanner = (p: {
    state: AuthState;
    onRetry: () => void;
    canRetry: boolean;
}): JSX.Element => (
    <div class="pre-launch-auth-panel-failed">
        <div class="pre-launch-auth-panel-failed-message">
            ✗ Auth failed: {p.state.error || "unknown error"}
        </div>
        <Button onClick={() => p.onRetry()} disabled={!p.canRetry}>
            Try again
        </Button>
    </div>
);

