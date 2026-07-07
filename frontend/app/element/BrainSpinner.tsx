// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BrainSpinner — the pulsating AgentMux brain mark as a reusable loading
 * indicator. The same asset/animation as the startup splash
 * (index.html's #startup-loading, frontend/app/init/startup-splash.ts), but
 * that one is a single-shot, non-reusable singleton overlay tied to one DOM
 * id — this component can be mounted per-pane instead, any number of times,
 * each fading independently via the `fading` prop.
 *
 * See docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md.
 */

import { atoms } from "@/store/global";
import type { JSX } from "solid-js";
import brainSvg from "@/app/asset/logo-brain.svg?raw";
import "./BrainSpinner.scss";

interface BrainSpinnerProps {
    /** Set true to cross-fade the spinner out. Caller owns unmounting it
     *  after the transition ends (e.g. via a short timeout matching the CSS
     *  duration) — this component only handles the visual fade. */
    fading?: boolean;
    class?: string;
}

export const BrainSpinner = (props: BrainSpinnerProps): JSX.Element => {
    const reducedMotion = atoms.prefersReducedMotionAtom;

    return (
        <div
            class={`brain-spinner${props.fading ? " is-fading" : ""}${reducedMotion() ? " is-reduced-motion" : ""}${props.class ? ` ${props.class}` : ""}`}
            aria-hidden="true"
            innerHTML={brainSvg}
        />
    );
};

BrainSpinner.displayName = "BrainSpinner";
