// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 1).

import { atoms } from "@/app/store/global";
import { ModalLayer } from "@/element/ModalLayer";
import { createEffect, createSignal, onCleanup, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { AgentPresentationView } from "./agent-view";
import { AgentPicker } from "./components/AgentPicker";
import { AgentHistoryTabView } from "./history/AgentHistoryTabView";
import { HISTORY_TAB_FOR_META_KEY } from "./open-history-tab";

// Matches BrainSpinner.scss's own `.is-fading` opacity transition duration —
// the AgentPicker->AgentPresentationView cross-fade (AgentBlockContent,
// below) reuses the same visual timing so the two fades feel like one brand
// moment rather than two differently-tuned animations back to back.
const PICKER_FADE_OUT_MS = 200;

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

    // Cross-fade AgentPicker -> AgentPresentationView instead of an instant
    // hard cut when this SAME block gains an agentId in place (launching an
    // agent from a blank "+" tab's picker — no block-stack mutation, no
    // node remount, so PR #2761's leaf reveal gate never covers this
    // transition at all). SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md
    // §2.3/§4 Option B.
    //
    // Same stuck-visible race as block.tsx's ready()-gate (see
    // docs/retro/retro-block-ready-gate-spinner-stuck-visible-race-2026-08-23.md):
    // seeding `pickerVisible` from `!agentId()` read once at construction,
    // then relying on `on(agentId, ..., {defer: true})`'s first (swallowed)
    // run to treat "agentId already set" as "nothing to do," is two
    // different reads of `agentId()` taken at two different times. If
    // `agentId()` resolves in the gap between them, the seed is never
    // corrected. Fixed the same way: the first observation and the seed
    // are now the same read, inside the same effect.
    const [pickerVisible, setPickerVisible] = createSignal(true);
    const [pickerFadingOut, setPickerFadingOut] = createSignal(false);
    let pickerFadeRaf: number | undefined;
    let pickerFadeTimeout: ReturnType<typeof setTimeout> | undefined;
    let pickerGateInitialized = false;
    onCleanup(() => {
        if (pickerFadeRaf !== undefined) cancelAnimationFrame(pickerFadeRaf);
        clearTimeout(pickerFadeTimeout);
    });
    createEffect(() => {
        const id = agentId();
        if (pickerFadeRaf !== undefined) cancelAnimationFrame(pickerFadeRaf);
        clearTimeout(pickerFadeTimeout);
        if (!pickerGateInitialized) {
            // First observation of `agentId()` for this mount: reflect it
            // directly, no fade — there's nothing painted yet to fade from
            // either way.
            pickerGateInitialized = true;
            setPickerVisible(!id);
            setPickerFadingOut(false);
            return;
        }
        if (id) {
            if (!pickerVisible()) return; // already past the transition
            // One rAF so the picker paints at full opacity at least once
            // before the fade starts — flipping straight to the
            // "is-fading" class in this same tick would apply opacity:0
            // on the very first paint, with nothing to visibly transition
            // from.
            pickerFadeRaf = requestAnimationFrame(() => setPickerFadingOut(true));
            pickerFadeTimeout = setTimeout(() => {
                setPickerVisible(false);
                setPickerFadingOut(false);
            }, PICKER_FADE_OUT_MS);
        } else {
            // Lost the agentId (not a normal path, but stay correct) —
            // show the picker again immediately, no fade needed going
            // this direction.
            setPickerFadingOut(false);
            setPickerVisible(true);
        }
    });

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
                        {/* Cross-fades out on top of AgentPresentationView
                            once agentId() is set, instead of the two Shows
                            above hard-swapping instantly — see
                            pickerVisible/pickerFadingOut above.
                            SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md §2.3. */}
                        <Show when={pickerVisible()}>
                            <div
                                class="agent-picker-host"
                                classList={{
                                    // Applied the instant agentId() is set
                                    // (same render as AgentPresentationView
                                    // appearing) so this never sits in
                                    // normal flow alongside it, even for
                                    // one frame.
                                    "is-overlay": !!agentId(),
                                    "is-fading": pickerFadingOut(),
                                    "is-reduced-motion": atoms.prefersReducedMotionAtom(),
                                }}
                            >
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
