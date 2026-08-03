// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMux Cloud connect — the single, shared implementation of the AgentMux
 * Cloud sign-in (Cognito hosted-UI PKCE). Backed by the `muxbus.login /
 * .status / .disconnect` RPCs and the singleton credential store
 * (`db_muxbus_credentials`, one global row). AgentMux Cloud is one app-wide
 * session — NOT a pluralizable IdentityAccount — so it never goes through the
 * generic service-OAuth path (`oauth_client.rs` / `account.oauth.*`).
 *
 * Two consumers share this module so there is exactly one implementation:
 *   - `MuxBusConnectSection` — the per-agent identity panel (AgentIdentityPanel).
 *   - `AgentMuxConnectPanel` — the Armory → Accounts gallery tile.
 *
 * The gallery additionally projects the signed-in session into the connected
 * list as a read-only row; that projection lives in `accounts-manager.tsx` and
 * reads the same controller (`useMuxBusStatus`) so connect/disconnect refresh
 * the tile, the row, and the panel together.
 *
 * See specs/archive/SPEC_TRUST_CENTER_2026_06_15.md and muxbus/pkce.rs.
 */

import { createSignal, onMount, Show, type Accessor, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ProviderLogo } from "@/element/ProviderLogo";
import { writeText as clipboardWriteText } from "@/util/clipboard";

// Production Cognito config — set after deployment.
// Override with VITE_MUXBUS_COGNITO_DOMAIN / VITE_MUXBUS_CLIENT_ID at build time.
const MUXBUS_COGNITO_DOMAIN =
    (import.meta.env.VITE_MUXBUS_COGNITO_DOMAIN as string | undefined) ??
    "https://muxbus-auth.auth.us-east-1.amazoncognito.com";
const MUXBUS_CLIENT_ID =
    (import.meta.env.VITE_MUXBUS_CLIENT_ID as string | undefined) ?? "";

export interface MuxBusStatus {
    connected: boolean;
    email: string;
    cognitoDomain: string;
    expiresAt: number;
    valid: boolean;
}

const DISCONNECTED: MuxBusStatus = {
    connected: false,
    email: "",
    cognitoDomain: "",
    expiresAt: 0,
    valid: false,
};

/**
 * Shared reactive controller for the AgentMux Cloud session. One instance is
 * the single source of truth for any UI that needs connection state — pass the
 * same controller to the gallery tile, the read-only row, and the connect
 * panel so a connect/disconnect updates all of them.
 */
export interface MuxBusController {
    status: Accessor<MuxBusStatus | null>;
    loading: Accessor<boolean>;
    error: Accessor<string | null>;
    /** Re-pull `muxbus.status`. Treats any error as "disconnected". */
    refresh: () => Promise<void>;
    /** Run the browser PKCE login (blocks up to 5 min), then refresh. */
    connect: () => Promise<void>;
    /** Clear stored credentials, then refresh. */
    disconnect: () => Promise<void>;
    /** False when no built-in client id is baked into this build. */
    isConfigured: () => boolean;
}

export function useMuxBusStatus(): MuxBusController {
    const [status, setStatus] = createSignal<MuxBusStatus | null>(null);
    const [loading, setLoading] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    const refresh = async () => {
        try {
            setStatus(await RpcApi.MuxBusStatusCommand(TabRpcClient));
        } catch {
            // no credentials or server not reachable — treat as disconnected
            setStatus({ ...DISCONNECTED });
        }
    };

    const connect = async () => {
        if (!MUXBUS_CLIENT_ID) {
            setError("AgentMux client ID not configured (contact AgentMux team).");
            return;
        }
        setError(null);
        setLoading(true);
        try {
            const result = await RpcApi.MuxBusLoginCommand(TabRpcClient, {
                cognitoDomain: MUXBUS_COGNITO_DOMAIN,
                clientId: MUXBUS_CLIENT_ID,
            });
            if (result.success) {
                await refresh();
            } else {
                setError(result.error ?? "Login failed.");
            }
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    const disconnect = async () => {
        setError(null);
        setLoading(true);
        try {
            await RpcApi.MuxBusDisconnectCommand(TabRpcClient);
            await refresh();
        } catch (e) {
            setError(String(e));
        } finally {
            setLoading(false);
        }
    };

    const isConfigured = () => MUXBUS_CLIENT_ID !== "";

    return { status, loading, error, refresh, connect, disconnect, isConfigured };
}

/** Human-readable local expiry, or null when not connected. */
function expiryLabel(status: MuxBusStatus | null): string | null {
    if (!status?.connected) return null;
    return new Date(status.expiresAt * 1000).toLocaleString();
}

/**
 * A connect/disconnect error can be long (e.g. a wrapped keychain error) —
 * wraps + scrolls instead of blowing up the panel, and offers a copy button
 * so the user can hand the exact text to support/an issue without retyping
 * it. `class` names the container; `-text` / `-copy-btn` suffixes get their
 * own rules alongside it (see `_identity-panel.scss` / `_form-overlay.scss`).
 */
function CopyableErrorMessage(props: { message: string; class: string }): JSX.Element {
    const [copied, setCopied] = createSignal(false);
    let copiedTimer: ReturnType<typeof setTimeout> | null = null;

    const copy = () => {
        void clipboardWriteText(props.message)
            .then(() => {
                if (copiedTimer) clearTimeout(copiedTimer);
                setCopied(true);
                copiedTimer = setTimeout(() => setCopied(false), 1500);
            })
            .catch(() => {});
    };

    return (
        <div class={props.class}>
            <span class={`${props.class}-text`}>{props.message}</span>
            <button
                type="button"
                class={`${props.class}-copy-btn`}
                onClick={copy}
                title={copied() ? "Copied!" : "Copy error message"}
                aria-label="Copy error message"
            >
                <i class={copied() ? "fa-solid fa-check" : "fa-solid fa-copy"} aria-hidden="true" />
            </button>
        </div>
    );
}

/**
 * Per-agent identity panel section (unchanged UI). Owns its own controller so
 * the agent panel keeps working independently of the Armory tab.
 */
export const MuxBusConnectSection = (): JSX.Element => {
    const muxbus = useMuxBusStatus();
    onMount(() => void muxbus.refresh());
    const { status, loading, error } = muxbus;

    return (
        <div class="agent-identity-muxbus">
            <div class="agent-identity-section-title">AgentMux Cloud</div>
            <Show
                when={status()?.connected}
                fallback={
                    <div class="agent-identity-muxbus-row">
                        <span class="agent-identity-none">Not connected</span>
                        <button
                            class="agent-identity-new-btn"
                            disabled={loading()}
                            onClick={() => void muxbus.connect()}
                        >
                            {loading() ? "Connecting…" : "Connect"}
                        </button>
                    </div>
                }
            >
                <div class="agent-identity-muxbus-row">
                    <div class="agent-identity-muxbus-info">
                        <span class="agent-identity-account-name">{status()!.email}</span>
                        <Show when={!status()!.valid}>
                            <span class="agent-identity-muxbus-expired"> (token expired)</span>
                        </Show>
                        <Show when={expiryLabel(status())}>
                            <span class="agent-identity-muxbus-expiry"> · expires {expiryLabel(status())}</span>
                        </Show>
                    </div>
                    <button
                        class="agent-identity-unassign-btn"
                        disabled={loading()}
                        title="Disconnect from AgentMux Cloud"
                        onClick={() => void muxbus.disconnect()}
                    >
                        Disconnect
                    </button>
                </div>
            </Show>
            <Show when={error()}>
                <CopyableErrorMessage class="agent-identity-error" message={error()!} />
            </Show>
        </div>
    );
};

MuxBusConnectSection.displayName = "MuxBusConnectSection";

/**
 * Armory → Accounts gallery connect panel. Renders in the existing
 * accounts-chooser modal shell (no new SCSS). Uses the controller owned by
 * `AccountsManager` so the tile/row/panel stay in sync.
 */
export function AgentMuxConnectPanel(props: {
    muxbus: MuxBusController;
    onClose: () => void;
}): JSX.Element {
    const { muxbus } = props;
    return (
        <div
            class="accounts-chooser-overlay"
            onClick={(e) => e.target === e.currentTarget && props.onClose()}
        >
            <div class="accounts-chooser" role="dialog" aria-label="Connect AgentMux">
                <div class="accounts-chooser-header">
                    <span class="account-tile-logo">
                        <ProviderLogo provider="agentmux" size={20} />
                    </span>
                    <span class="accounts-chooser-title">Connect AgentMux</span>
                    <button
                        type="button"
                        class="accounts-chooser-close"
                        onClick={() => props.onClose()}
                        aria-label="Close"
                    >
                        ✕
                    </button>
                </div>
                <div class="accounts-chooser-modes">
                    <Show when={muxbus.error()}>
                        <CopyableErrorMessage class="identity-form-error" message={muxbus.error()!} />
                    </Show>

                    <Show
                        when={muxbus.isConfigured()}
                        fallback={
                            <div class="oauth-byo-note">
                                ⓘ AgentMux Cloud sign-in isn’t configured in this build (client ID
                                missing). Contact the AgentMux team.
                            </div>
                        }
                    >
                        <Show
                            when={muxbus.status()?.connected}
                            fallback={
                                <>
                                    <div class="oauth-byo-note">
                                        ⓘ Sign in to AgentMux Cloud in your browser — one account per
                                        install, no key to manage.
                                    </div>
                                    <div class="identity-key-actions">
                                        <button
                                            class="identity-btn identity-btn-primary"
                                            disabled={muxbus.loading()}
                                            onClick={() => void muxbus.connect()}
                                        >
                                            {muxbus.loading() ? "Connecting…" : "Connect with AgentMux"}
                                        </button>
                                    </div>
                                </>
                            }
                        >
                            <div class="oauth-byo-note">
                                Connected as{" "}
                                <strong>{muxbus.status()!.email || "AgentMux Cloud"}</strong>
                                <Show when={!muxbus.status()!.valid}> (token expired)</Show>
                            </div>
                            <div class="identity-key-actions">
                                <button
                                    class="identity-btn identity-btn-secondary"
                                    disabled={muxbus.loading()}
                                    onClick={() => void muxbus.disconnect()}
                                >
                                    Disconnect
                                </button>
                            </div>
                        </Show>
                    </Show>
                </div>
            </div>
        </div>
    );
}

AgentMuxConnectPanel.displayName = "AgentMuxConnectPanel";
