// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSeatPanel — the "Seat…" modal, opened from My Agents' row menu and
 * the agent pane's right-click menu: the agent's role, qualifier, lead and area,
 * set only by the operator through `agent.setSeat`
 * (docs/specs/SPEC_AGENT_SEATS_IN_THE_UI_2026_10_23.md §3). Also hosts
 * "Import roster…" (SeatRosterImport).
 *
 * The seat shown is the one `listagents` gives: the stored seat, or a spawned
 * worker's effective one (`set_at_ms === 0`), which the tab marks "from
 * spawn" so the operator can give it a real one.
 */

import { Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import "./AgentPrimitiveModal.scss";
import "./AgentSeatPanel.scss";
import { RosterRow, SeatEditor, SeatIntro } from "./AgentSeatPanelParts";
import { createAgentSeat, type AgentSeatPanelProps } from "./AgentSeatPanelState";

export { createSeatForm, NO_SHARED_DEFINITION_NOTE, type SeatForm } from "./AgentSeatPanelState";

export const AgentSeatPanel = (props: AgentSeatPanelProps): JSX.Element => {
    const s = createAgentSeat(props);
    return (
        <div class="agent-primitive-modal-detail agent-seat-panel" data-testid="agent-seat-panel">
            <div class="agent-primitive-modal-readonly">
                <SeatIntro s={s} agentName={props.agentName} />
                <Show when={s.self()}>
                    <SeatEditor s={s} />
                </Show>
                <RosterRow s={s} />
                <Show when={props.onClose}>
                    {(close) => (
                        <div class="agent-primitive-modal-actions">
                            <Button onClick={() => close()()}>Close</Button>
                        </div>
                    )}
                </Show>
            </div>
        </div>
    );
};

AgentSeatPanel.displayName = "AgentSeatPanel";
