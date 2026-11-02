// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The picker's loading cover: held until the definitions and My agents have both loaded once, then faded.
// Split out of AgentPicker.tsx (#13044 S6).

import { trackPaneContent } from "@/app/store/pane-content-holds";
import { scaledMs } from "@/app/motion/motion";
import { DURATION_BASE_MS } from "@/app/motion/tokens";
import { createPaneReadiness } from "@/app/store/pane-readiness";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { useWindowTabDisplayed } from "@/app/workspace/window-tab-visibility";
import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from "solid-js";

export function usePickerReadiness(
    blockId: string,
    agents: Accessor<AgentDefinition[]>,
    definitionsLoading: Accessor<boolean>
) {
    // Combined "picker content is ready to reveal" gate — true once BOTH
    // independent async sources feeding this view have resolved their
    // FIRST load: the definitions list (definitionsLoading above) and
    // MyAgentsList's own ListRecentSessionsCommand resource (reported back
    // via onFirstLoad below, since MyAgentsList owns that resource
    // internally). Neither alone was sufficient — gating only on
    // definitionsLoading still let MyAgentsList's own empty `<ul>` flash
    // before its rows arrived; gating only on MyAgentsList's load left the
    // OUTER `<Show when={agents().length > 0}>` free to flash the "No
    // definitions configured" empty state first for users who actually
    // have agents, simply because `agents()` starts at `[]` and is
    // indistinguishable from "genuinely zero" without definitionsLoading.
    // One overlay, held until both are done, replaces two independent
    // partial-content flashes with a single reveal — same principle as
    // agent-view.tsx's pane-level loading overlay (one gate, one reveal),
    // applied here across two sibling data sources instead of one.
    const [myAgentsLoaded, setMyAgentsLoaded] = createSignal(false);
    // A genuinely-empty definitions list never mounts MyAgentsList at all (it
    // lives inside the `agents().length > 0` branch below) — so `loaded`
    // must not wait on `myAgentsLoaded` in that case, or the overlay would
    // spin forever for a user with zero agent definitions.
    const pickerReady = createMemo(() => {
        if (definitionsLoading()) return false;
        if (agents().length === 0) return true;
        return myAgentsLoaded();
    });

    // Hold-then-fade, now expressed through the shared readiness controller
    // (SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md phase 2). Mechanics are
    // unchanged — one gate, released when `pickerReady()` flips, then one CSS fade
    // duration before unmount so the overlay's removal is never a pop.
    //
    // What changes is ownership: this component used to render
    // `.agent-pane-loading-overlay` itself, with its own ready signal and its own
    // unmount timer, while agent-view.tsx rendered the SAME class with its own —
    // two covers for one pane, whichever unmounted last winning. Both now go
    // through <PaneLoadingCover>, so the class has a single owner.
    const windowTabDisplayed = useWindowTabDisplayed();
    // The picker's loading is all behind this cover (pane-content-holds.ts).
    onCleanup(trackPaneContent(blockId));
    const pickerReadiness = createPaneReadiness({
        label: "agent-picker",
        holdFor: blockId,
        hidden: () => !windowTabDisplayed(),
    });
    const releasePickerGate = pickerReadiness.gate("picker-content");
    let pickerOverlayFadeTimeout: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(pickerOverlayFadeTimeout));
    createEffect(() => {
        if (pickerReady()) releasePickerGate();
    });
    createEffect(() => {
        if (pickerReadiness.phase() === "revealing") {
            pickerOverlayFadeTimeout = setTimeout(() => pickerReadiness.revealComplete(), scaledMs(DURATION_BASE_MS) + 20); // the cover's scaled fade, as usePaneReveal
        }
    });

    return { pickerReadiness, setMyAgentsLoaded };
}
