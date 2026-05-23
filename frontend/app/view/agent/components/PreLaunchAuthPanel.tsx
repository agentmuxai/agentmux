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
import type { ProviderDefinition } from "../providers";
import "./PreLaunchAuthPanel.scss";

export interface PreLaunchAuthPanelProps {
    /** The provider to authenticate. */
    provider: ProviderDefinition | undefined;
    /** Currently-selected identity bundle id, or "" if the user hasn't
     *  picked one yet. The empty case triggers the Connect CTA so OAuth
     *  can mint a fresh bundle. */
    identityId: Accessor<string>;
    /** True when the selected non-blank bundle has a binding for the
     *  agent's provider. When false (or while bindings are still
     *  loading), the panel routes to `needs-bundle` so the user goes
     *  through the OAuth flow before Launch enables. */
    hasMatchingBinding: Accessor<boolean>;
    /** Status of the bound account for this (bundle, provider) pair,
     *  or `null` when there's no matching binding. Per spec §4.4 the
     *  oauth-class canonical values are `"valid" | "expired" |
     *  "needs_reauth" | "unknown"`. When the panel sees `"expired"` /
     *  `"needs_reauth"` together with `hasMatchingBinding=true`, the
     *  Connect CTA's wording shifts from "Connect to <Provider>" to
     *  "<Provider> credentials need reconnecting" — same OAuth flow,
     *  just a clearer nudge. Optional: callers that don't surface
     *  account status fall through to the generic wording.
     *
     *  Note: this does NOT change the launch-gate. A non-blank bundle
     *  with a matching binding still counts toward `hasMatchingBinding`
     *  even if its status is `expired` / `needs_reauth`; the agent CLI
     *  will trigger its own OAuth refresh on first call. The wording
     *  exists to surface the situation so the user can act proactively
     *  via the Reconnect button in the bundle manager. */
    bindingStatus?: Accessor<string | null>;
    /** Auth flow controller — owned by the Launch modal so its
     *  lifetime spans the whole modal, not just the panel's
     *  conditional mount. Lifting fixed the "memory change forgot
     *  login" bug where a brief re-render unmounted this panel and
     *  destroyed an internally-constructed controller along with it.
     *  See docs/specs/SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md. */
    controller: AuthFlowController;
    /** Called when a new bundle is created via OAuth or API-key
     *  submission — parent should refresh the identity list and
     *  switch the dropdown to this new bundle. */
    onBundleCreated: (bundleId: string) => void;
    /** Called when the user clicks Connect and the outcome is
     *  `needs-bundle` (no identity bundle selected — a genuinely
     *  fresh OAuth). Instead of starting OAuth immediately the panel
     *  asks the parent to interpose the New Identity modal so the
     *  user names the bundle first; OAuth then resumes against the
     *  named bundle. When omitted the panel falls back to starting
     *  OAuth directly (legacy behaviour). Spec
     *  SPEC_BUNDLE_MANAGEMENT_2026_05_22.md §2. */
    onRequestNewIdentity?: () => void;
    /** When true, the panel fires its OAuth `startConnect()` exactly
     *  once on mount — used by the New Identity → launch round-trip so
     *  the user doesn't have to click Connect a second time after
     *  naming the bundle. Spec SPEC_BUNDLE_MANAGEMENT_2026_05_22.md §2.
     *  The auto-start only fires from a connect-able state
     *  (`unauthenticated` / `expired`); a panel that already shows
     *  `ready` / `waiting` is left alone. */
    autoStartAuth?: boolean;
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

    // Forward `succeeded` / `api-key-accepted` to bundle-creation
    // callback so the modal swaps its Identity selection to the
    // newly-persisted bundle id. Skip placeholder ids
    // (`pending-bundle-for-<sid>`) — those are pre-persistence
    // synthetic ids; the real id arrives via the next state
    // transition.
    createEffect(() => {
        const s = controller.state();
        if (
            s.kind === "ready" &&
            s.bundleId &&
            !s.bundleId.startsWith("pending-bundle-for-")
        ) {
            props.onBundleCreated(s.bundleId);
        }
    });

    // When the identity dropdown changes, drive the controller's
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
        const id = props.identityId();
        const prov = props.provider;
        // Track hasMatchingBinding so the effect re-fires when
        // bindings finish loading — without this, a freshly created
        // "+ New" bundle that initially reports `hasMatchingBinding=
        // false` would never re-dispatch to `ready` once the user
        // connects an account.
        const hasBinding = props.hasMatchingBinding();
        if (!prov) return;
        // "" id (no bundle selected) tells the controller / backend
        // to create a fresh bundle as part of the OAuth flow.
        untrack(() => controller.selected(prov.id, id, outcomeFor(id, prov.id, hasBinding)));
    });

    // Connect / Retry click handler. The OAuth invariant per
    // SPEC_OAUTH_IDENTITY_BUNDLES_2026_05_22.md §4.5 is that OAuth
    // never starts without a bundle id in hand — tokens always land
    // INSIDE a known bundle dir, never ambient. So:
    //   - empty identityId → route to New Identity modal first; OAuth
    //     resumes against the new bundle on the autoStartAuth round
    //     trip. The parent's `onRequestNewIdentity` owns the
    //     tabModal.replace chain into that modal.
    //   - empty identityId AND no `onRequestNewIdentity` callback →
    //     refuse to start OAuth. This is a guard against a future
    //     parent forgetting to wire the callback; a silent ambient-
    //     creds OAuth would re-open the §4.5 hole this PR closes.
    //     Surface a failed state so the user sees something instead
    //     of a dead Connect button.
    //   - non-empty identityId → straight to OAuth. Reused for
    //     `needs-account`, `expired`, and openclaw's force-fresh
    //     `needs-bundle` (where outcomeFor() returns `needs-bundle`
    //     even with a bundle selected — the gate is on the actual
    //     identityId, not the outcome).
    const handleConnect = (): void => {
        const prov = props.provider;
        if (!prov) return;
        if (props.identityId() === "") {
            if (props.onRequestNewIdentity) {
                props.onRequestNewIdentity();
                return;
            }
            // OAuth-without-bundle invariant — no quiet fallback to
            // ambient OAuth (would defeat the per-bundle isolation
            // PR C wires). Bail visibly.
            console.warn(
                "[auth-diag] handleConnect: empty identityId and no onRequestNewIdentity callback — refusing to start OAuth (PR C invariant)",
            );
            controller.failConnect(
                new Error(
                    "OAuth requires a named Identity bundle. Click '+ New identity' first.",
                ),
            );
            return;
        }
        void startConnect(controller, prov);
    };

    // Auto-start OAuth once on mount when the parent set `autoStartAuth`
    // — the New Identity → launch round-trip (spec §2). By this point
    // the user has named + created the bundle, so the dropdown carries
    // a non-blank `identityId` and `outcomeFor()` resolves to
    // `needs-account`; `handleConnect()` therefore routes straight to
    // OAuth (NOT back into the New Identity modal) with `intoBundleId`
    // = the new bundle. Guarded so it fires exactly once and only from
    // a connect-able state — a re-render must not re-trigger it, and a
    // panel that's already `waiting`/`ready` is left untouched.
    let autoStartFired = false;
    createEffect(() => {
        if (!props.autoStartAuth || autoStartFired) return;
        const kind = controller.state().kind;
        if (kind !== "unauthenticated" && kind !== "expired") return;
        autoStartFired = true;
        untrack(() => handleConnect());
    });

    // No onCleanup here — controller lifetime is owned by the parent
    // (AgentLaunchModal). Disposing on panel unmount would destroy
    // in-flight auth state any time the parent's `<Show when={authRequired()}>`
    // briefly flips false (the root cause of the "memory change forgot
    // login" bug fixed in this PR).

    return (
        <div class="pre-launch-auth-panel">
            <Switch>
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
                        bindingStatus={props.bindingStatus?.() ?? null}
                        hasBinding={props.hasMatchingBinding()}
                        onConnect={() => handleConnect()}
                        disabled={props.disabled ?? false}
                    />
                </Match>
            </Switch>
        </div>
    );
};

/** Compute the SelectionOutcome from the bundle id and its binding
 *  state. Treat:
 *   - blank singleton or "+ New"-just-created (no matching binding)
 *     → `needs-bundle` (Connect creates new or attaches a binding).
 *   - non-blank bundle WITH a binding for this provider → `ready`. */
function outcomeFor(
    identityId: string,
    providerId: string | undefined,
    hasMatchingBinding: boolean,
): SelectionOutcome {
    // Phase α for openclaw: bundle persistence isn't wired yet, so an
    // already-selected identity can't be trusted as authenticated. Force
    // a fresh OAuth on every launch so AgentLaunchModal's openclaw
    // override actually gates Launch. Lift once Phase δ wires real
    // bundle storage.
    if (providerId === "openclaw") return "needs-bundle";
    if (!identityId) return "needs-bundle";
    // Reagent + codex P1 on PR #910 round 3 — a non-blank bundle
    // without a binding for the agent's provider can't supply creds
    // (e.g. "+ New identity" created an empty "Work" bundle that the
    // user hasn't connected yet). Use `needs-account` (NOT
    // `needs-bundle`) so the reducer preserves `intoBundleId` and
    // OAuth lands on THIS bundle instead of creating a fresh one
    // (codex P1 round 4 — `needs-bundle` clears the bundle id).
    if (!hasMatchingBinding) return "needs-account";
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
    /** `IdentityAccount.status` of the bound row for this (bundle,
     *  provider) pair, or `null` when no binding exists yet. Drives
     *  the spec §4.4 status-aware wording when the user has a
     *  binding but its tokens are `expired` / `needs_reauth`. */
    bindingStatus: string | null;
    /** True when `hasMatchingBinding(state, provider)` is true — gates
     *  the reconnect-wording branch so it only triggers when there IS
     *  a binding to refresh (the no-binding case keeps the generic
     *  Connect CTA wording the user already knows). */
    hasBinding: boolean;
    onConnect: () => void;
    disabled: boolean;
}): JSX.Element => {
    const catalog = () =>
        p.provider ? getCliCatalogEntry(p.provider.id) : undefined;
    const providerLabel = () => p.provider?.displayName ?? "this provider";
    const isExpired = () => p.state.kind === "expired";
    // Spec §4.4 — when the user has a binding for this provider but
    // its account status flags a reconnect (expired / needs_reauth),
    // shift to the "credentials need reconnecting" wording. Same
    // Connect button, same OAuth flow underneath; just a clearer nudge
    // so the user understands they're refreshing a known login rather
    // than starting fresh.
    const needsReconnect = () =>
        p.hasBinding &&
        (p.bindingStatus === "needs_reauth" || p.bindingStatus === "expired");
    return (
        <div class="pre-launch-auth-panel-cta">
            <div class="pre-launch-auth-panel-warning">
                {needsReconnect()
                    ? `⚠ Your ${providerLabel()} credentials need reconnecting.`
                    : isExpired()
                        ? `⚠ Your ${providerLabel()} session is expired. Re-authenticate before launching.`
                        : `⚠ ${providerLabel()} requires an OAuth login before launch.`}
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
                    ? `Re-runs OAuth into your existing Identity bundle. Launch stays available — your CLI will refresh on first call.`
                    : `Opens browser → ${providerLabel()} login → returns to AgentMux.
                    Tokens get saved into a new Identity bundle so the next
                    agent doesn't have to re-authenticate.`}
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

