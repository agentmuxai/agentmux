// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentRuntimeDropup — single trigger + floating panel consolidating the
 * Mode / Model / Effort drop-ups that used to be three separate FlyoutMenu
 * pills in AgentComposerStrip (SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02
 * Fix 7). One button shows a live "Mode · Model · Effort" summary; the panel
 * floats upward from it, grouped into three labeled sections, and stays open
 * across selections so one visit can touch all three axes (deliberate
 * departure from FlyoutMenu's close-on-select — SPEC §9.2).
 *
 * Renders through AnchoredPopover (positioning, chrome zoom, the airspace
 * cut) rather than FlyoutMenu: FlyoutMenu only renders a flat MenuItem[] list
 * and has no concept of grouped sections with headers. Its own keyboard and
 * focus handling stay here; AnchoredPopover's dismiss is not used.
 *
 * Model options stay registry-driven via getProvider(providerId)?.models
 * (live-overlaid from the providers.models RPC, same as the prior three-pill
 * implementation) so an API-sourced catalog surfaces new labels automatically.
 *
 * Also closes via an explicit button (top-right of the panel) in addition to
 * Esc/outside-click/re-clicking the trigger.
 *
 * Its size never follows what is selected or running: the status line, the
 * running-model line and the five effort rows are always there, each in a
 * fixed space, and the trigger holds its width while the panel is open. A
 * change made in the panel can't move the rows under the pointer
 * (SPEC_REMOTES_INTO_CONNECTORS_2026_10_08.md §6).
 *
 * Spec: docs/specs/SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md,
 * docs/specs/SPEC_AGENT_RUNTIME_DROPUP_CLOSE_BUTTON_2026_08_07.md,
 * docs/specs/SPEC_REMOTES_INTO_CONNECTORS_2026_10_08.md §6.
 */

import type { JSX } from "solid-js";
import { useRuntimeAgreement } from "./runtime-dropup-agreement";
import { useDropupListeners } from "./runtime-dropup-listeners";
import { useModelMigration } from "./runtime-dropup-migration";
import type { AgentRuntimeDropupProps } from "./runtime-dropup-options";
import { createDropupActions } from "./runtime-dropup-rows";
import { createDropupState } from "./runtime-dropup-state";
import { DropupView } from "./runtime-dropup-view";

/**
 * The dropup is a state object (runtime-dropup-state.ts) and a run of hook groups that act on it, called in
 * the order their effects must be created: the process agreement, the model migration, the open-panel listeners.
 */
export const AgentRuntimeDropup = (props: AgentRuntimeDropupProps): JSX.Element => {
    const s = createDropupState(props);
    const a = useRuntimeAgreement(s);
    useModelMigration(s);
    const act = createDropupActions(s, a);
    useDropupListeners(s, act);

    return <DropupView s={s} a={a} act={act} />;
};

AgentRuntimeDropup.displayName = "AgentRuntimeDropup";
