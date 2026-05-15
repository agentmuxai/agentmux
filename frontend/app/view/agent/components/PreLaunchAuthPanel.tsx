// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PreLaunchAuthPanel — the Connect-with-OAuth UI block that sits
 * inline in `AgentLaunchModal` before the Launch button. Owns an
 * `AuthFlowController` instance for the lifetime of the modal.
 *
 * Spec: `docs/specs/SPEC_PRE_LAUNCH_OAUTH_FLOW_2026_05_14.md` §3 + §8.
 *
 * Renders four mutually-exclusive panels off `controller.state().kind`:
 *   - `unauthenticated` / `expired` — Connect CTA (OAuth or API-key)
 *   - `waiting` — "Waiting for OAuth…" with URL fallback + Cancel
 *   - `ready` — green check banner ("Connected as <email>")
 *   - `failed` — red error banner + retry CTA
 *
 * Exposes the controller's `state` accessor to the parent so the
 * Launch button can gate on `state().kind === "ready"`.
 */

import { Button } from "@/element/button";
import { getApi } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    createEffect,
    createSignal,
    Match,
    onCleanup,
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
    /** Currently-selected identity bundle id. "blank" = singleton. */
    identityId: Accessor<string>;
    /** Notify parent when auth state changes — used to gate Launch. */
    onStateChange: (state: AuthState) => void;
    /** Called when a new bundle is created via OAuth or API-key
     *  submission — parent should refresh the identity list and
     *  switch the dropdown to this new bundle. */
    onBundleCreated: (bundleId: string) => void;
    /** Inline-disabled flag mirroring the modal's submitting state.
     *  Prevents starting a Connect mid-launch. */
    disabled?: boolean;
}

export const PreLaunchAuthPanel = (props: PreLaunchAuthPanelProps): JSX.Element => {
    const controller = new AuthFlowController();

    // Mirror controller.state() through createEffect → parent prop.
    createEffect(() => {
        props.onStateChange(controller.state());
    });

    // Forward `succeeded` / `api-key-accepted` to bundle-creation
    // callback so the modal can refresh the bundle list.
    //
    // Codex P2 on #847 (round 7): skip placeholder bundle ids the
    // OAuth backend synthesizes pre-PR-C (`pending-bundle-for-<sid>`).
    // Selecting one as identityId would launch against a row that
    // doesn't exist in wstore. Until PR C-2 wires real persistence,
    // OAuth-success leaves the dropdown on blank — the user's session
    // is still authenticated, the launch path treats blank as
    // "create-on-launch" via the existing flow.
    createEffect(() => {
        const s = controller.state();
        if (
            s.kind === "ready" &&
            s.bundleId &&
            s.bundleId !== "blank" &&
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
        if (!prov) return;
        // Codex P2 on #847 round 8: "blank" is a UI sentinel meaning
        // "no bundle selected", not a real bundleId. Normalize it to
        // "" before handing to the controller so `auth.start` /
        // `auth.submitapikey` receive `intoBundleId: undefined`
        // (= "create new bundle") rather than `"blank"` (= "attach
        // to bundle named blank", which doesn't exist in wstore).
        const bundleArg = id === "blank" ? "" : id;
        untrack(() => controller.selected(prov.id, bundleArg, outcomeFor(id)));
    });

    onCleanup(() => {
        controller.dispose();
    });

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
                        keeps the user informed in the interim. */}
                    <AuthenticatedStub state={controller.state()} />
                </Match>
                <Match when={controller.state().kind === "failed"}>
                    <FailedBanner
                        state={controller.state()}
                        onRetry={() => void startConnect(controller, props.provider)}
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
                        onConnect={() => void startConnect(controller, props.provider)}
                        disabled={props.disabled ?? false}
                    />
                </Match>
            </Switch>
        </div>
    );
};

/** Compute the SelectionOutcome from the bundle id. The richer
 *  expired / needs-account computation (per-bundle binding lookup)
 *  is deferred to PR B-4 / PR D — for MVP we treat:
 *   - blank singleton → `needs-bundle` (Connect creates new)
 *   - non-blank bundle → `ready` (trust prior auth — old behavior). */
function outcomeFor(identityId: string): SelectionOutcome {
    if (!identityId || identityId === "blank") return "needs-bundle";
    return "ready";
}

async function startConnect(
    controller: AuthFlowController,
    provider: ProviderDefinition | undefined,
): Promise<void> {
    if (!provider) return;
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
    } catch (e) {
        // Surface the real ResolveCli error in the FailedBanner —
        // reagent P1 on #847: previously this discarded `e` and
        // called connect with an empty cliPath, producing a
        // misleading backend "CLI not found at ''" message.
        controller.failConnect(e);
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
        } catch (e) {
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
    if (controller.state().closed) return;
    await controller.connect({
        cliPath,
        authLoginArgs: provider.authLoginCommand,
        authCheckArgs: provider.authCheckCommand,
        authEnv,
    });
}

// ── Sub-panels ─────────────────────────────────────────────────────

const ConnectCta = (p: {
    provider: ProviderDefinition | undefined;
    state: AuthState;
    onConnect: () => void;
    disabled: boolean;
}): JSX.Element => {
    const catalog = () =>
        p.provider ? getCliCatalogEntry(p.provider.id) : undefined;
    const providerLabel = () => p.provider?.displayName ?? "this provider";
    const isExpired = () => p.state.kind === "expired";
    return (
        <div class="pre-launch-auth-panel-cta">
            <div class="pre-launch-auth-panel-warning">
                {isExpired()
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
                    {isExpired() ? "Re-authenticate" : `Connect to ${providerLabel()}`}
                </span>
            </Button>
            <div class="pre-launch-auth-panel-hint">
                Opens browser → {providerLabel()} login → returns to AgentMux.
                Tokens get saved into a new Identity bundle so the next
                agent doesn't have to re-authenticate.
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
                            void navigator.clipboard.writeText(p.state.authUrl);
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

const AuthenticatedStub = (p: { state: AuthState }): JSX.Element => (
    <div class="pre-launch-auth-panel-ready">
        ✓ Authenticated{p.state.email ? ` as ${p.state.email}` : ""}.
        {" "}Awaiting bundle save…
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

