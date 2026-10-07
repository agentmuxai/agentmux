// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDevicesPanel — the "Devices" tab inside AgentStashModal: one checkbox,
 * "Hide from paired devices". A hidden agent is left out of what a paired
 * device lists and can't be watched from one; an open feed of it closes
 * (agentmux-mobile's SPEC_AGENT_STATUS_AND_LIVE_PANE_FEED_2026_10_07 §4.3).
 * Stored on the agent's definition (`db_agents.hide_from_devices`) through
 * `viewer.agent-hidden`, and saved as soon as it is changed.
 */

import { createSignal, Show, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import "./AgentPrimitiveModal.scss";

interface AgentDevicesPanelProps {
    agentId: string;
}

export const AgentDevicesPanel = (props: AgentDevicesPanelProps): JSX.Element => {
    const [hidden, setHidden] = createSignal(false);
    const [loaded, setLoaded] = createSignal(false);
    const [saving, setSaving] = createSignal(false);
    const [error, setError] = createSignal<string | null>(null);

    void RpcApi.ViewerAgentHiddenCommand(TabRpcClient, { agent_id: props.agentId })
        .then((r) => setHidden(r.hidden))
        .catch((e) => setError(String(e)))
        .finally(() => setLoaded(true));

    const handleChange = async (next: boolean): Promise<void> => {
        setError(null);
        setSaving(true);
        try {
            const r = await RpcApi.ViewerAgentHiddenCommand(TabRpcClient, { agent_id: props.agentId, hidden: next });
            setHidden(r.hidden);
        } catch (e) {
            setError(String(e));
        } finally {
            setSaving(false);
        }
    };

    return (
        <div class="agent-primitive-modal-detail">
            <div class="agent-primitive-modal-readonly">
                <span class="agent-primitive-modal-field-label">Paired devices</span>
                <p class="agent-primitive-modal-global-note">
                    Devices paired with this computer (the host menu's "Pair a device") can watch every agent here,
                    read-only. Hide this agent to keep it off them.
                </p>
                <Show when={error()}>
                    <div class="agent-primitive-modal-error">{error()}</div>
                </Show>
                <label class="agent-primitive-modal-bind-row">
                    <input
                        type="checkbox"
                        checked={hidden()}
                        disabled={!loaded() || saving()}
                        onChange={(e) => void handleChange(e.currentTarget.checked)}
                    />
                    <span>Hide from paired devices</span>
                </label>
            </div>
        </div>
    );
};

AgentDevicesPanel.displayName = "AgentDevicesPanel";
