// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a new agent should default to a container, and what to tell the user
 * when the container's image can't be had.
 *
 * Container is the preferred runtime, but only when it can actually start: the
 * CLI supports it, Docker answers, and the image is not known to be refused or
 * missing. `unknown` (no answer from the registry) never counts against it, so a
 * proxy or an offline check does not take away a working default. What the person
 * last chose themselves comes before the template's suggestion: an explicit Native
 * wins over a container default, and an explicit Container is kept for a template
 * that suggests Native, as long as it can run.
 *
 * Spec: docs/specs/SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md section 3.4.
 */

import type { ContainerImageAccess } from "@/types/rpc/ContainerImageAccess";

/** `undefined` while the check has not answered yet. */
export type ContainerImageState = ContainerImageAccess | undefined;

/** True when the registry definitely refuses the image or does not have it. */
export function imageBlocksContainer(image: ContainerImageState): boolean {
    return image === "denied" || image === "not_found";
}

export interface PreselectInput {
    /** The template's own suggested runtime (`AgentDefinition.agent_type`). */
    templateType: string | undefined;
    /** Does this provider's CLI have a container image at all? */
    containerSupported: boolean;
    /** Does the Docker daemon answer? */
    dockerAvailable: boolean;
    image: ContainerImageState;
    /** The runtime the person last chose explicitly in the new-agent form, if any. */
    lastChoice?: "host" | "container" | null;
}

/**
 * Whether to preselect the container runtime. False until the image check has
 * answered, so the choice is made once, with the answer in hand.
 */
export function shouldPreselectContainer(input: PreselectInput): boolean {
    if (input.lastChoice === "host") return false;
    if (input.lastChoice !== "container" && input.templateType !== "container") return false;
    if (!input.containerSupported || !input.dockerAvailable) return false;
    if (input.image === undefined) return false;
    return !imageBlocksContainer(input.image);
}

/** The line to show under the runtime picker, or null when there is nothing to say. */
export function containerImageNote(image: ContainerImageState): string | null {
    switch (image) {
        case "denied":
            return "The container image can't be downloaded: its registry refuses access. New agents run as Native agents (on this computer) until that is fixed.";
        case "not_found":
            return "The container image wasn't found in its registry. New agents run as Native agents (on this computer) until that is fixed.";
        default:
            return null;
    }
}
