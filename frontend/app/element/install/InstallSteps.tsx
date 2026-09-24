// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Layer 1 of the universal install dialog
 * (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §4.1): one row per step with
 * a status icon, a plain-language label, an optional right-aligned hint, and
 * one muted subline for the active or failed step.
 *
 * Icons are text glyphs with an accessible status name, so colour is never
 * the only signal.
 */

import { For, Show, type JSX } from "solid-js";

import type { InstallStep, InstallStepStatus } from "./install-types";
import "./install-steps.scss";

const ICON: Record<InstallStepStatus, string> = {
    pending: "○",
    active: "◐",
    done: "✓",
    failed: "✗",
    skipped: "–",
};

const STATUS_NAME: Record<InstallStepStatus, string> = {
    pending: "Not started",
    active: "In progress",
    done: "Done",
    failed: "Failed",
    skipped: "Skipped",
};

export const InstallSteps = (props: { steps: InstallStep[] }): JSX.Element => (
    <ol class="install-steps" aria-label="Install steps">
        <For each={props.steps}>
            {(step) => (
                <li
                    class="install-step"
                    data-status={step.status}
                    aria-current={step.status === "active" ? "step" : undefined}
                >
                    <span class="install-step-icon" role="img" aria-label={STATUS_NAME[step.status]}>
                        {ICON[step.status]}
                    </span>
                    <span class="install-step-label">{step.label}</span>
                    <Show when={step.hint}>
                        <span class="install-step-hint">{step.hint}</span>
                    </Show>
                    <Show when={step.subline}>
                        <span class="install-step-subline">{step.subline}</span>
                    </Show>
                </li>
            )}
        </For>
    </ol>
);
