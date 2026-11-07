// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentPicker — shown when an agent pane has no agentId in block meta.
 *
 * Two-tier layout (Phase 1 — SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md):
 *  - **My Agents** (top): user-owned agents (`is_seeded = 0`). Each
 *    row shows the agent's current Option E session state (preview,
 *    node count, last-active timestamp). Click = reattach via the
 *    existing `continueOfInstanceId + workDirOverride` flow.
 *  - **+ New from template** (bottom): seeded templates
 *    (`is_seeded = 1`). Click = open its panel below the card's row
 *    (`AgentCreatePanel`), which clones the template into a new user
 *    agent and launches it (SPEC_AGENT_PICKER_INLINE_CREATE_2026_10_18.md).
 *
 * Templates by Phase 1 invariant carry NO session zone (the startup
 * migration `migrate_promote_template_sessions_v1` evicts any
 * pre-existing template-session into a user agent). The template
 * cards therefore never show the "+ New" pill or auto-continue.
 *
 * Pre-existing flows kept intact:
 *  - Modifier-key force-modal on a My Agents row (Shift / Ctrl / Alt /
 *    Cmd) bypasses auto-continue.
 *  - The tools a harness needs and its CLI's install come first, as
 *    steps of the same panel, when they apply.
 *
 * See docs/specs/SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md (legacy)
 * and docs/specs/SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md (current).
 */

import { Show, type JSX } from "solid-js";
import { useAgentDefinitions, useOpenDefinitionMap } from "./agent-definition-hooks";
import { EmptyPicker, PickerContent } from "./agent-picker-parts";
import { createAgentPicker, type AgentPickerProps } from "./agent-picker-state";

export { useAgentDefinitions, useOpenDefinitionMap };

// ── AgentPicker component ───────────────────────────────────────────────────────

export const AgentPicker = (props: AgentPickerProps): JSX.Element => {
    const s = createAgentPicker(props);

    return (
        <>
            <Show when={s.agents().length > 0} fallback={<EmptyPicker s={s} />}>
                <PickerContent s={s} props={props} />
            </Show>
        </>
    );
};

AgentPicker.displayName = "AgentPicker";
