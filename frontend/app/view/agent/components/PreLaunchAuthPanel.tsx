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
import { seedGlobalLogin } from "../flows/seed-global-login";
import type { ProviderDefinition } from "../providers";
import type { LogFn } from "../types";
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

export const PreLaunchAuthPanel = (props: PreLaunchAuthPanelProps): JSX.Element => {
    // Controller is owned by the parent (AgentLaunchModal); panel
    // mount/unmount doesn't construct or dispose it. The parent
    // reads `controller.state()` directly for `authStateKind()`
    // — no more `onStateChange` callback wiring.
    const controller = props.controller;

    // Auto-open the OAuth URL in the user's default browser as soon as
    // the auth controller surfaces it. Same effect as the legacy
    // launch-flow.ts inline path. The URL also stays visible in the
    // WaitingPanel below with a Copy button — that's the manual
    // fallback when the OS doesn't route the open (browser closed,
    // protocol handler missing, etc.). Fires once per URL (the guard
    // tracks the last URL we opened so re-renders don't re-fire).
    let lastOpenedUrl: string | null = null;
    createEffect(() => {
        const s = controller.state();
        const url = s.authUrl;
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
    // (SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md §4.5's "OAuth never
    // starts without a bundle id" invariant) — that invariant no
    // longer applies; the per-account isolation dir is resolved
    // server-side regardless of whether an account was pre-selected.
    const handleConnect = (): void => {
        const prov = props.provider;
        if (!prov) return;
        void startConnect(controller, prov);
    };

    // "Use my existing login" — the PRIMARY path for Claude v2.1.x, whose
    // in-app OAuth can't open a browser when WE spawn it (a dead end —
    // SPEC_HOST_CLI_LOGIN_CAPTURE §0). Copies the user's valid GLOBAL login
    // into the agent's isolated dir, then marks the controller `ready` so
    // Launch enables — no in-app browser, no OAuth.
    const seedLog: LogFn = (_cat, msg) => console.log(`[auth-diag] seed: ${msg}`);
    const handleUseExistingLogin = async (): Promise<void> => {
        const prov = props.provider;
        if (!prov) return;
        // Resolve the agent's isolated auth dir so the seed lands where the
        // agent reads it (host falls back to the shared dir if absent/invalid).
        let configDir: string | undefined;
        if (prov.authConfigDirEnvVar) {
            try {
                configDir = await getApi().ensureAuthDir(prov.id);
            } catch (e) {
                console.warn(
                    `[auth-diag] seed ensureAuthDir failed: ${(e as Error)?.message ?? String(e)}`,
                );
            }
        }
        const ok = await seedGlobalLogin(prov.id, seedLog, configDir);
        if (ok) {
            controller.markSeeded(props.accountId());
        } else {
            controller.failConnect(
                new Error(
                    "No valid global Claude login to copy. Run `claude setup-token` in a real terminal (it opens a browser), complete it, then click “Use my existing login” again.",
                ),
            );
        }
    };

    // No onCleanup here — controller lifetime is owned by the parent
    // (AgentLaunchModal). Disposing on panel unmount would destroy
    // in-flight auth state any time the parent's `<Show when={authRequired()}>`
    // briefly flips false (the root cause of the "memory change forgot
    // login" bug fixed in this PR).

    return (
        <div class="pre-launch-auth-panel">
            <Switch>
                {/* An account the backend's expiry probe flagged stale
                    (`needs_reauth` / `expired`) lands the controller in
                    `ready` because outcomeFor() only looks at "does the
                    account supply this provider?". Without this arm the
                    panel would render <ReadyBanner /> and the
                    reconnect wording in ConnectCta would never be
                    seen — the user would have no signal their
                    credentials need refreshing. Match BEFORE the
                    generic ready arm so the reconnect CTA wins.
                    Clicking Connect re-runs OAuth into the SAME
                    account's isolation dir (existingAccountId in
                    auth-flow-controller.ts's connect()), refreshing the
                    token in place. codex P1 on #982. */}
                <Match
                    when={
                        controller.state().kind === "ready" &&
                        (props.accountStatus?.() === "needs_reauth" ||
                            props.accountStatus?.() === "expired")
                    }
                >
                    <ConnectCta
                        provider={props.provider}
                        state={controller.state()}
                        accountStatus={props.accountStatus?.() ?? null}
                        hasAccount={props.accountSuppliesProvider()}
                        onConnect={() => handleConnect()}
                        canSeed={props.provider?.id === "claude"}
                        onUseExistingLogin={() => void handleUseExistingLogin()}
                        disabled={props.disabled ?? false}
                    />
                </Match>
                <Match when={controller.state().kind === "ready"}>
                    <ReadyBanner />
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
                <Match
                    when={
                        controller.state().kind === "unauthenticated" ||
                        controller.state().kind === "expired"
                    }
                >
                    <ConnectCta
                        provider={props.provider}
                        state={controller.state()}
                        accountStatus={props.accountStatus?.() ?? null}
                        hasAccount={props.accountSuppliesProvider()}
                        onConnect={() => handleConnect()}
                        canSeed={props.provider?.id === "claude"}
                        onUseExistingLogin={() => void handleUseExistingLogin()}
                        disabled={props.disabled ?? false}
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
    /** True for providers where seed-from-global is the path (Claude
     *  v2.1.x — in-app OAuth can't open a browser under our spawn, so the
     *  Connect CTA is a dead end). When true the panel leads with "Use my
     *  existing login" instead of OAuth. SPEC_HOST_CLI_LOGIN_CAPTURE §0. */
    canSeed: boolean;
    onUseExistingLogin: () => void;
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
            <Show
                when={p.canSeed}
                fallback={
                    <>
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
                    </>
                }
            >
                {/* Claude v2.1.x: in-app OAuth can't open a browser under our
                    spawn (SPEC_HOST_CLI_LOGIN_CAPTURE §0), so seed the user's
                    existing global login instead — the upstream-recommended
                    method (issue #7100). PRIMARY path. */}
                <Button
                    onClick={() => p.onUseExistingLogin()}
                    disabled={p.disabled}
                    className="pre-launch-auth-panel-connect green solid"
                >
                    <span class="pre-launch-auth-panel-connect-icon" aria-hidden="true">🌐</span>
                    <span class="pre-launch-auth-panel-connect-label">Use my existing login</span>
                </Button>
                <div class="pre-launch-auth-panel-hint">
                    Copies your existing terminal login into this agent — no in-app
                    browser needed. First time? Run <code>claude setup-token</code> in a
                    terminal, finish it in the browser, then click this.
                </div>
            </Show>
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

