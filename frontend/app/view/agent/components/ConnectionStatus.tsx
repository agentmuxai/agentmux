// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * ConnectionStatus - Provider-aware connection state
 *
 * Auth flow (Claude/OAuth providers):
 *   1. Agent model runs `claude auth status --json` → checks loggedIn
 *   2. If not logged in → shows "Not Authenticated" + Login button
 *   3. Login button → agent model spawns `claude auth login` in PTY
 *   4. User completes browser auth → process exits → model re-checks
 *   5. If logged in → model spawns session CLI
 *
 * Auth flow (Gemini/Codex/API-key providers):
 *   User enters API key → saved to provider store → passed as env var
 */

import { useAtomValue, useSetAtom, atom as jotaiAtom } from "jotai";
import React, { memo, useCallback, useState } from "react";
import { getApi } from "@/app/store/global";
import type { PrimitiveAtom } from "jotai";
import type { AuthState, UserInfo } from "../types";
import { PROVIDERS } from "../providers";

// Fallback atom for when providerConfigAtom is not provided
const fallbackProviderConfigAtom: PrimitiveAtom<ProviderConfig | null> = jotaiAtom<ProviderConfig | null>(null);

interface ConnectionStatusProps {
    authAtom: PrimitiveAtom<AuthState>;
    userInfoAtom: PrimitiveAtom<UserInfo | null>;
    providerConfigAtom?: PrimitiveAtom<ProviderConfig | null>;
    onRestart?: () => void;
    onStartLogin?: () => void;
}

export const ConnectionStatus: React.FC<ConnectionStatusProps> = memo(
    ({ authAtom, userInfoAtom, providerConfigAtom, onRestart, onStartLogin }) => {
        const authState = useAtomValue(authAtom);
        const userInfo = useAtomValue(userInfoAtom);
        const providerConfig = useAtomValue(providerConfigAtom ?? fallbackProviderConfigAtom);
        const setAuthState = useSetAtom(authAtom);
        const setUserInfo = useSetAtom(userInfoAtom);

        const currentProvider = providerConfig?.default_provider || "claude";
        const providerDef = PROVIDERS[currentProvider];
        const authType = providerDef?.authType || "oauth";

        const handleDisconnect = useCallback(async () => {
            try {
                await getApi().clearProviderAuth(currentProvider);
                setAuthState({ status: "disconnected" });
                setUserInfo(null);
            } catch (error) {
                setAuthState({ status: "error", error: `Disconnect failed: ${String(error)}` });
            }
        }, [currentProvider, setAuthState, setUserInfo]);

        const handleRetry = useCallback(() => {
            if (onRestart) {
                onRestart();
            }
        }, [onRestart]);

        // --- Connected ---
        if (authState.status === "connected" && userInfo) {
            return (
                <div className="agent-connection-status connected">
                    <div className="connection-icon">{"\u2713"}</div>
                    <div className="connection-info">
                        <div className="connection-label">
                            Connected to {providerDef?.displayName || currentProvider}
                        </div>
                        <div className="connection-email">{userInfo.email}</div>
                    </div>
                    <button className="connection-disconnect-btn" onClick={handleDisconnect} title="Disconnect">
                        <i className="fa fa-sign-out" />
                    </button>
                </div>
            );
        }

        // --- Connecting (auth check in progress or login in progress) ---
        if (authState.status === "connecting") {
            return (
                <div className="agent-connection-status connecting">
                    <div className="connection-icon">{"\u23F3"}</div>
                    <div className="connection-info">
                        <div className="connection-label">Authenticating...</div>
                        <div className="connection-hint">
                            {authType === "oauth"
                                ? "Complete login in your browser"
                                : "Validating API key..."}
                        </div>
                    </div>
                </div>
            );
        }

        // --- Error ---
        if (authState.status === "error") {
            return (
                <div className="agent-connection-status error">
                    <div className="connection-icon">{"\u26A0\uFE0F"}</div>
                    <div className="connection-info">
                        <div className="connection-label">Authentication Failed</div>
                        {authState.error && <div className="connection-error">{authState.error}</div>}
                    </div>
                    <button className="connection-retry-btn" onClick={handleRetry}>
                        <i className="fa fa-refresh" /> Retry
                    </button>
                </div>
            );
        }

        // --- Disconnected: show provider-appropriate auth UI ---
        if (authType === "api-key") {
            return <ApiKeyInput provider={currentProvider} providerDef={providerDef} onAuth={setAuthState} />;
        }

        // OAuth (Claude) — user must run `claude auth login`
        return (
            <div className="agent-connection-status disconnected">
                <div className="connection-message">
                    <div className="connection-title">
                        {providerDef?.displayName || currentProvider} — Not Authenticated
                    </div>
                    <div className="connection-description">
                        Click Login to authenticate via your browser.
                        The CLI will open a login page automatically.
                    </div>
                </div>
                <button className="connection-connect-btn" onClick={onStartLogin}>
                    <i className="fa fa-sign-in" /> Login
                </button>
            </div>
        );
    }
);

ConnectionStatus.displayName = "ConnectionStatus";

// --- API Key Input component for Gemini/Codex ---

const ApiKeyInput: React.FC<{
    provider: string;
    providerDef: any;
    onAuth: (state: AuthState) => void;
}> = memo(({ provider, providerDef, onAuth }) => {
    const [apiKey, setApiKey] = useState("");
    const [saving, setSaving] = useState(false);

    const handleSave = useCallback(async () => {
        if (!apiKey.trim()) return;
        setSaving(true);
        try {
            await getApi().setProviderAuth(provider, apiKey.trim());
            onAuth({ status: "connected" });
        } catch (error) {
            onAuth({ status: "error", error: `Failed to save API key: ${String(error)}` });
        } finally {
            setSaving(false);
        }
    }, [provider, apiKey, onAuth]);

    return (
        <div className="agent-connection-status disconnected">
            <div className="connection-message">
                <div className="connection-title">
                    {providerDef?.displayName || provider} API Key
                </div>
                <div className="connection-description">
                    Enter your API key to authenticate.
                </div>
            </div>
            <div className="connection-apikey-form">
                <input
                    type="password"
                    className="connection-apikey-input"
                    placeholder="Enter API key..."
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    onKeyDown={(e) => {
                        if (e.key === "Enter") void handleSave();
                    }}
                />
                <button
                    className="connection-connect-btn"
                    onClick={handleSave}
                    disabled={!apiKey.trim() || saving}
                >
                    {saving ? "Saving..." : "Save"}
                </button>
            </div>
        </div>
    );
});

ApiKeyInput.displayName = "ApiKeyInput";
