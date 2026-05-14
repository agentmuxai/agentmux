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
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import {
    createEffect,
    createMemo,
    createSignal,
    Match,
    onCleanup,
    Show,
    Switch,
    type Accessor,
    type JSX,
} from "solid-js";

import {
    AuthFlowController,
    type AuthState,
    type SelectionOutcome,
} from "../auth";
import type { ProviderDefinition } from "../providers";

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
    // The parent re-renders the Launch button on every state change.
    createEffect(() => {
        props.onStateChange(controller.state());
    });

    // Forward `succeeded` / `api-key-accepted` to bundle-creation
    // callback so the modal can refresh the bundle list.
    createEffect(() => {
        const s = controller.state();
        if (s.kind === "ready" && s.bundleId && s.bundleId !== "blank") {
            props.onBundleCreated(s.bundleId);
        }
    });

    // When the identity dropdown changes, drive the controller's
    // `selected` action. The outcome is computed below.
    createEffect(() => {
        const id = props.identityId();
        const prov = props.provider;
        if (!prov) return;
        controller.selected(prov.id, id, outcomeFor(id));
    });

    onCleanup(() => {
        controller.dispose();
    });

    return (
        <div class="pre-launch-auth-panel">
            <Switch>
                <Match when={controller.state().kind === "ready"}>
                    <ReadyBanner state={controller.state()} />
                </Match>
                <Match when={controller.state().kind === "waiting"}>
                    <WaitingPanel
                        state={controller.state()}
                        onCancel={() => void controller.cancel()}
                        onSubmitCallback={(url) => void controller.submitCallback(url)}
                    />
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
                    <Show
                        when={props.provider?.authType === "api-key"}
                        fallback={
                            <ConnectCta
                                provider={props.provider}
                                state={controller.state()}
                                onConnect={() =>
                                    void startConnect(controller, props.provider)
                                }
                                disabled={props.disabled ?? false}
                            />
                        }
                    >
                        <ApiKeyCta
                            provider={props.provider}
                            onSubmit={(key, accountName) =>
                                void controller.submitApiKey(key, accountName)
                            }
                            disabled={props.disabled ?? false}
                        />
                    </Show>
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
    await controller.connect({
        cliPath,
        authLoginArgs: provider.authLoginCommand,
        authCheckArgs: provider.authCheckCommand,
        authEnv: provider.authExtraEnv,
    });
}

// ── Sub-panels ─────────────────────────────────────────────────────

const ConnectCta = (p: {
    provider: ProviderDefinition | undefined;
    state: AuthState;
    onConnect: () => void;
    disabled: boolean;
}): JSX.Element => (
    <div class="pre-launch-auth-panel-cta">
        <div class="pre-launch-auth-panel-warning">
            ⚠ {p.provider?.displayName ?? "This agent"} requires an OAuth login before launch.
        </div>
        <Button
            onClick={() => p.onConnect()}
            disabled={p.disabled}
            class="pre-launch-auth-panel-connect"
        >
            🔐 Connect with OAuth
        </Button>
        <div class="pre-launch-auth-panel-hint">
            Opens browser → {p.provider?.displayName ?? "provider"} login → returns to AgentMux.
            Tokens get saved into a new Identity bundle so the next agent
            doesn't have to re-authenticate.
        </div>
    </div>
);

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
                <div class="pre-launch-auth-panel-url-row">
                    <span class="pre-launch-auth-panel-url-label">
                        URL not opening? Copy this and paste anywhere:
                    </span>
                    <input
                        class="pre-launch-auth-panel-url-input"
                        readOnly
                        value={p.state.authUrl}
                        onClick={(e) => e.currentTarget.select()}
                    />
                    <Button
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

const ReadyBanner = (p: { state: AuthState }): JSX.Element => (
    <div class="pre-launch-auth-panel-ready">
        ✓ Connected. Ready to launch.
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

const ApiKeyCta = (p: {
    provider: ProviderDefinition | undefined;
    onSubmit: (apiKey: string, accountName: string) => void;
    disabled: boolean;
}): JSX.Element => {
    const [key, setKey] = createSignal("");
    const [accountName, setAccountName] = createSignal("default");
    const canSubmit = createMemo(() => key().trim().length > 0 && !p.disabled);
    return (
        <div class="pre-launch-auth-panel-apikey">
            <div class="pre-launch-auth-panel-warning">
                ⚠ {p.provider?.displayName ?? "This agent"} requires an API key before launch.
            </div>
            <input
                class="pre-launch-auth-panel-url-input"
                type="password"
                placeholder="Paste your API key…"
                value={key()}
                onInput={(e) => setKey(e.currentTarget.value)}
                disabled={p.disabled}
            />
            <input
                class="pre-launch-auth-panel-url-input"
                placeholder="Account name (for the new bundle)"
                value={accountName()}
                onInput={(e) => setAccountName(e.currentTarget.value)}
                disabled={p.disabled}
            />
            <Button
                onClick={() => p.onSubmit(key().trim(), accountName().trim() || "default")}
                disabled={!canSubmit()}
            >
                Save API key
            </Button>
            <div class="pre-launch-auth-panel-hint">
                The key is validated against{" "}
                <code>{(p.provider?.authCheckCommand ?? []).join(" ")}</code> before saving.
            </div>
        </div>
    );
};
