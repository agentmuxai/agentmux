// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneLoadingCover — the one component that renders `.agent-pane-loading-overlay`.
 *
 * Phase 2 of `docs/specs/SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md`. That class
 * was previously rendered from TWO places with independent lifecycles —
 * `agent-view.tsx` and `AgentPicker.tsx` — each with its own ready signal, its own
 * 220ms unmount timer, and no knowledge of the other. Whichever unmounted last won.
 * Both now render this.
 *
 * Driven by a `PaneReadinessPhase` rather than a pair of booleans, so "covered",
 * "fading" and "gone" cannot disagree:
 *
 *   assembling → mounted, opaque
 *   revealing  → mounted, fading (CSS transition in _loading-overlay.scss)
 *   live       → unmounted
 *
 * The caller owns the `revealing` → `live` transition (it knows the fade duration
 * its stylesheet uses) — see `PaneReadiness.revealComplete`.
 */

import { BrainSpinner } from "@/app/element/BrainSpinner";
import { atoms } from "@/app/store/global";
import type { PaneReadinessPhase } from "@/app/store/pane-readiness";
import { Show, type JSX } from "solid-js";

export interface PaneLoadingCoverProps {
    /** Current readiness phase for the pane being covered. */
    phase: () => PaneReadinessPhase;
}

export function PaneLoadingCover(props: PaneLoadingCoverProps): JSX.Element {
    const fading = () => props.phase() === "revealing";
    return (
        <Show when={props.phase() !== "live"}>
            <div
                class="agent-pane-loading-overlay"
                classList={{
                    "is-fading": fading(),
                    "is-reduced-motion": atoms.prefersReducedMotionAtom(),
                }}
            >
                <BrainSpinner fading={fading()} />
            </div>
        </Show>
    );
}
