// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ConnectionStatus - Provider-aware connection state
 */

import { getApi } from "@/app/store/global";
import { createSignal, Show, type JSX } from "solid-js";
import type { SignalPair } from "../state";
import { PROVIDERS } from "../providers";
import type { AuthState, UserInfo } from "../types";

interface ConnectionStatusProps {
    authAtom: SignalPair<AuthState>;
    userInfoAtom: SignalPair<UserInfo | null>;
    providerConfigAtom?: SignalPair<ProviderConfig | null>;
    onRestart?: () => void;
    onStartLogin?: () => void;
}

export const ConnectionStatus = ({
    authAtom,
    userInfoAtom,
    providerConfigAtom,
    onRestart,
    onStartLogin,
}: ConnectionStatusProps): JSX.Element => {
    const [authState, setAuthState] = authAtom;
    const [userInfo, setUserInfo] = userInfoAtom;
    const [providerConfig] = providerConfigAtom ?? createSignal<ProviderConfig | null>(null);

    const currentProvider = () => providerConfig()?.default_provider || "claude";
    const providerDef = () => PROVIDERS[currentProvider()];
    const authType = () => providerDef()?.authType || "oauth";

    const handleDisconnect = async () => {
        try {
            await getApi().clearProviderAuth(currentProvider());
            setAuthState({ status: "disconnected" });
            setUserInfo(null);
        } catch (error) {
            setAuthState({ status: "error", error: `Disconnect failed: ${String(error)}` });
        }
    };

    const handleRetry = () => {
        if (onRestart) {
            onRestart();
        }
    };

    // --- Connected ---
    if (authState().status === "connected" && userInfo()) {
        return (
            <div class="agent-connection-status connected">
                <div class="connection-icon">{"\u2713"}</div>
                <div class="connection-info">
                    <div class="connection-label">
                        Connected to {providerDef()?.displayName || currentProvider()}
                    </div>
                    <div class="connection-email">{userInfo().email}</div>
                </div>
                <button class="connection-disconnect-btn" onClick={handleDisconnect} title="Disconnect">
                    <i class="fa fa-sign-out" />
                </button>
            </div>
        );
    }

    // --- Connecting ---
    if (authState().status === "connecting") {
        return (
            <div class="agent-connection-status connecting">
                <div class="connection-icon">{"\u23F3"}</div>
                <div class="connection-info">
                    <div class="connection-label">Authenticating...</div>
                    <div class="connection-hint">
                        {authType() === "oauth"
                            ? "Complete login in your browser"
                            : "Validating API key..."}
                    </div>
                </div>
            </div>
        );
    }

    // --- Error ---
    if (authState().status === "error") {
        return (
            <div class="agent-connection-status error">
                <div class="connection-icon">{"\u26A0\uFE0F"}</div>
                <div class="connection-info">
                    <div class="connection-label">Authentication Failed</div>
                    <Show when={(authState() as any).error}>
                        <div class="connection-error">{(authState() as any).error}</div>
                    </Show>
                </div>
                <button class="connection-retry-btn" onClick={handleRetry}>
                    <i class="fa fa-refresh" /> Retry
                </button>
            </div>
        );
    }

    // --- Disconnected: show provider-appropriate auth UI ---
    if (authType() === "api-key") {
        return <ApiKeyInput provider={currentProvider()} providerDef={providerDef()} onAuth={setAuthState} />;
    }

    // OAuth (Claude) — user must run `claude auth login`
    return (
        <div class="agent-connection-status disconnected">
            <div class="connection-message">
                <div class="connection-title">
                    {providerDef()?.displayName || currentProvider()} — Not Authenticated
                </div>
                <div class="connection-description">
                    Click Login to authenticate via your browser.
                    The CLI will open a login page automatically.
                </div>
            </div>
            <button class="connection-connect-btn" onClick={onStartLogin}>
                <i class="fa fa-sign-in" /> Login
            </button>
        </div>
    );
};

ConnectionStatus.displayName = "ConnectionStatus";

// --- API Key Input component for Gemini/Codex ---

const ApiKeyInput = ({
    provider,
    providerDef,
    onAuth,
}: {
    provider: string;
    providerDef: any;
    onAuth: (state: AuthState) => void;
}): JSX.Element => {
    const [apiKey, setApiKey] = createSignal("");
    const [saving, setSaving] = createSignal(false);

    const handleSave = async () => {
        if (!apiKey().trim()) return;
        setSaving(true);
        try {
            await getApi().setProviderAuth(provider, apiKey().trim());
            onAuth({ status: "connected" });
        } catch (error) {
            onAuth({ status: "error", error: `Failed to save API key: ${String(error)}` });
        } finally {
            setSaving(false);
        }
    };

    return (
        <div class="agent-connection-status disconnected">
            <div class="connection-message">
                <div class="connection-title">
                    {providerDef?.displayName || provider} API Key
                </div>
                <div class="connection-description">
                    Enter your API key to authenticate.
                </div>
            </div>
            <div class="connection-apikey-form">
                <input
                    type="password"
                    class="connection-apikey-input"
                    placeholder="Enter API key..."
                    value={apiKey()}
                    onInput={(e) => setApiKey((e.target as HTMLInputElement).value)}
                    onKeyDown={(e) => {
                        if (e.key === "Enter") void handleSave();
                    }}
                />
                <button
                    class="connection-connect-btn"
                    onClick={handleSave}
                    disabled={!apiKey().trim() || saving()}
                >
                    {saving() ? "Saving..." : "Save"}
                </button>
            </div>
        </div>
    );
};

ApiKeyInput.displayName = "ApiKeyInput";
