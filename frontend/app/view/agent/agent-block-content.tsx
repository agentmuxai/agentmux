// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 1).

import { ModalLayer } from "@/element/ModalLayer";
import { Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { AgentPresentationView } from "./agent-view";
import { AgentPicker } from "./components/AgentPicker";
import { AgentHistoryTabView } from "./history/AgentHistoryTabView";
import { HISTORY_TAB_FOR_META_KEY } from "./open-history-tab";

/**
 * Content half of the agent pane — becomes `AgentViewModel.viewComponent`.
 * Switches between the agent picker and the live presentation view (or the
 * read-only history reader). Constructed fresh per stack member, exactly
 * like every other `viewComponent` — the "one instance, one immutable
 * blockId for its lifetime" `ViewModel` contract is unchanged here.
 *
 * The pane-scope `<ModalLayer>` wrap lives HERE, not in `AgentPaneChrome`.
 * Chrome does now mount for every agent pane, so this is no longer
 * load-bearing the way it was when chrome was stack-size-gated — but it
 * stays here deliberately: the launch picker (`useModalLayer()`, opened
 * before any agentId exists) belongs to CONTENT, so wrapping at the content
 * root keeps the layer's lifetime tied to the thing that opens modals
 * rather than to chrome. Wrapping here covers both the
 * pre-launch picker AND the post-launch presentation view, and (once
 * `AgentPaneChrome` does mount) sits inside it, so the pane-scope lock
 * still holds across the entire pane lifecycle either way.
 * SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md.
 */
export const AgentBlockContent = ({ model }: { model: AgentViewModel }): JSX.Element => {
    const block = model.blockAtom;
    const agentId = () => block()?.meta?.["agentId"];
    // A block opened as a read-only history reader (openOrFocusHistoryTab)
    // — takes priority over the live/picker gate below, and never toggles
    // back: closing this reading posture is closing the tab, not swapping
    // content in place. See SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.1.
    const isHistoryTab = () => !!block()?.meta?.[HISTORY_TAB_FOR_META_KEY];

    // Launching an agent from this pane's picker swaps the picker out at once.
    // AgentPresentationView mounts behind its own opaque loading cover (the
    // logo), which then fades to the conversation: logo, then final content.
    // The picker used to cross-fade out on top of it for 200 ms instead, so
    // the picker's text showed through and dissolved into the logo, a flash
    // of text the owner asked to remove (2026-10-05). That fade was added
    // against a hard cut to a blank pane (SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md
    // §2.3), before the presentation view had its own cover from its first frame.

    // ReAgent P2 on SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md's
    // PR: owned by the model (one useAgentDefinitions() subscription per
    // ViewModel instance) instead of a fresh call here, so AgentPaneChrome
    // can read the SAME list via nodeModel.activeViewModel() instead of
    // independently subscribing a second time — see AgentViewModel.agentDefinitions'
    // own doc comment (agent-model.ts).
    const agentDefinitions = model.agentDefinitions;

    return (
        <ModalLayer scope="pane">
            <Show
                when={isHistoryTab()}
                fallback={
                    <>
                        <Show when={agentId()}>
                            <AgentPresentationView
                                model={model}
                                agentId={agentId()}
                                agentDefinitions={agentDefinitions}
                                progressBarMount={model.progressBarMount}
                            />
                        </Show>
                        <Show when={!agentId()}>
                            <div class="agent-picker-host">
                                <AgentPicker model={model} />
                            </div>
                        </Show>
                    </>
                }
            >
                {/* No progressBarMount here — a history tab is a read-only
                    reader with no live turn/working state of its own, so
                    there's nothing for a progress bar to represent. */}
                <AgentHistoryTabView model={model} />
            </Show>
        </ModalLayer>
    );
};

AgentBlockContent.displayName = "AgentBlockContent";
