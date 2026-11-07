// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentContainerLimitsPanel: the "Container limits…" modal, opened from a container agent's pane menu and from the
 * "Raise the limit" button on an out-of-memory row. A container agent's CPU and memory limits, what is in force now
 * and what Docker has (docs/specs/SPEC_CONTAINER_AGENT_RESOURCE_LIMITS_2026_10_29.md §3). Saving puts the new limits
 * on the running container at once: nothing restarts.
 */

import type { JSX } from "solid-js";

import { cpuNotes, memoryNotes } from "../container-limits";
import { AgentContainerCaches } from "./AgentContainerCaches";
import { LimitRow, LimitsActions, LimitsMessages } from "./agent-container-limits-parts";
import { createContainerLimits, type AgentContainerLimitsPanelProps } from "./agent-container-limits-state";
import "./AgentContainerLimitsPanel.scss";
import "./AgentPrimitiveModal.scss";

export const AgentContainerLimitsPanel = (props: AgentContainerLimitsPanelProps): JSX.Element => {
    const s = createContainerLimits(props);

    return (
        <div class="agent-primitive-modal-detail agent-container-limits-panel" data-testid="agent-container-limits-panel">
            <div class="agent-primitive-modal-readonly">
                <p class="agent-primitive-modal-global-note">
                    How much of Docker's memory and CPU {props.agentName}'s container may use. A change reaches the
                    running container at once; nothing restarts.
                </p>
                <LimitRow
                    s={s}
                    label="Memory"
                    draft={s.memory}
                    set={s.setMemory}
                    suffix="GB"
                    notes={() => memoryNotes(s.memory(), s.limits()?.effective_memory_mb, s.limits()?.daemon_memory_mb)}
                    testid="container-limits-memory"
                />
                <LimitRow
                    s={s}
                    label="CPUs"
                    draft={s.cpus}
                    set={s.setCpus}
                    suffix="CPUs"
                    notes={() => cpuNotes(s.cpus(), s.limits()?.effective_cpus, s.limits()?.daemon_cpus)}
                    testid="container-limits-cpus"
                />
                <AgentContainerCaches agentId={props.agentId} agentName={props.agentName} />
                <LimitsMessages s={s} />
                <LimitsActions s={s} onClose={props.onClose} />
            </div>
        </div>
    );
};

AgentContainerLimitsPanel.displayName = "AgentContainerLimitsPanel";
