// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import React, { memo, useCallback, useState } from "react";
import type { AgentViewModel } from "./agent-model";
import { getProviderList, type ProviderDefinition } from "./providers";
import "./agent-view.scss";

/**
 * Top-level wrapper: passes connectWithProvider into the provider picker.
 */
export const AgentViewWrapper: React.FC<ViewComponentProps<AgentViewModel>> = memo(({ model }) => {
    return <AgentProviderPicker onConnectWithProvider={model.connectWithProvider} />;
});

AgentViewWrapper.displayName = "AgentViewWrapper";

const PROVIDER_ICONS: Record<string, string> = {
    claude: "\u2728", // sparkles
    codex: "\uD83E\uDD16", // robot
    gemini: "\uD83D\uDC8E", // gem
};

const ProviderButton: React.FC<{
    provider: ProviderDefinition;
    onSelect: (providerId: string) => void;
    disabled: boolean;
}> = ({ provider, onSelect, disabled }) => {
    return (
        <button
            className="agent-provider-btn"
            onClick={() => onSelect(provider.id)}
            disabled={disabled}
        >
            <span className="agent-provider-icon">{PROVIDER_ICONS[provider.id] || "\u26A1"}</span>
            <span className="agent-provider-name">{provider.displayName}</span>
        </button>
    );
};

interface AgentProviderPickerProps {
    onConnectWithProvider: (providerId: string, cliPath: string) => Promise<void>;
}

/**
 * Provider selection screen. Clicking a button switches the block to a terminal.
 */
const AgentProviderPicker: React.FC<AgentProviderPickerProps> = memo(({ onConnectWithProvider }) => {
    const [launching, setLaunching] = useState<string | null>(null);
    const providers = getProviderList();

    const handleProviderSelect = useCallback(
        async (providerId: string) => {
            const provider = getProviderList().find((p) => p.id === providerId);
            if (!provider) return;
            setLaunching(providerId);
            try {
                await onConnectWithProvider(providerId, provider.cliCommand);
            } catch {
                setLaunching(null);
            }
        },
        [onConnectWithProvider]
    );

    return (
        <div className="agent-view">
            <div className="agent-document">
                <div className="agent-empty">
                    <div className="agent-connect-header">Connect</div>
                    <div className="agent-provider-buttons">
                        {providers.map((provider) => (
                            <ProviderButton
                                key={provider.id}
                                provider={provider}
                                onSelect={handleProviderSelect}
                                disabled={launching != null}
                            />
                        ))}
                    </div>
                    {launching && (
                        <div className="agent-install-status">
                            Launching {launching}...
                        </div>
                    )}
                </div>
            </div>
        </div>
    );
});

AgentProviderPicker.displayName = "AgentProviderPicker";
