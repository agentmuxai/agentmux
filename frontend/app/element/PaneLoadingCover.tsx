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
 *   revealing  → mounted, fading (CSS transition in PaneLoadingCover.scss)
 *   live       → unmounted
 *
 * The caller owns the `revealing` → `live` transition (it knows the fade duration
 * its stylesheet uses) — see `PaneReadiness.revealComplete`.
 */

import { BrainSpinner } from "@/app/element/BrainSpinner";
import { atoms } from "@/app/store/global";
import type { PaneReadinessPhase } from "@/app/store/pane-readiness";
import { Show, type JSX } from "solid-js";
import "./PaneLoadingCover.scss";

export interface PaneLoadingCoverProps {
    /** Current readiness phase for the pane being covered. */
    phase: () => PaneReadinessPhase;
    /**
     * Whether there is mounted content to sit on top of. Default `true`.
     *
     * Callers whose pane content is already mounted underneath (agent-view,
     * AgentPicker) leave this alone and get the absolute overlay. A caller that
     * covers a pane whose content has NOT mounted yet — block-level, before
     * `ready()` — passes `false` for that window, because there is no
     * positioned ancestor to resolve `inset: 0` against and the cover would
     * otherwise collapse or escape its pane depending on the render route.
     * Stating it beats inheriting it from the cascade (spec §3.2/§5.3).
     */
    overlay?: () => boolean;
}

export function PaneLoadingCover(props: PaneLoadingCoverProps): JSX.Element {
    const fading = () => props.phase() === "revealing";
    const overlay = () => props.overlay?.() ?? true;
    return (
        <Show when={props.phase() !== "live"}>
            <div
                class="agent-pane-loading-overlay"
                classList={{
                    "is-in-flow": !overlay(),
                    "is-fading": fading(),
                    "is-reduced-motion": atoms.prefersReducedMotionAtom(),
                }}
            >
                <BrainSpinner fading={fading()} />
            </div>
        </Show>
    );
}
