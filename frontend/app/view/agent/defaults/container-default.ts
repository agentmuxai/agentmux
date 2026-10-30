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

/**
 * The image choices of the new-agent form for a container agent: the standard image (the template's own), or the
 * developer image (Rust, a C toolchain, go-task and a headless display, for building software).
 * Spec: docs/specs/SPEC_CONTAINER_AGENT_DEVELOPER_IMAGE_2026_10_29.md section 4.
 */
export type ContainerImageChoice = "standard" | "developer";

/** What a stored `agent:container_image` of `:developer` means: this AgentMux version's developer image. */
export const DEVELOPER_CONTAINER_IMAGE = "ghcr.io/agentmuxai/agent-base:developer";

/** The image to ask for at creation: the developer image when chosen, nothing (the template's own) otherwise. */
export function containerImageForChoice(runtime: "host" | "container", choice: ContainerImageChoice): string | undefined {
    return runtime === "container" && choice === "developer" ? DEVELOPER_CONTAINER_IMAGE : undefined;
}

/** The line to show under the image choice when the developer image can't be had, or null. */
export function developerImageNote(image: ContainerImageState): string | null {
    switch (image) {
        case "denied":
            return "The developer image can't be downloaded: its registry refuses access. Pick the standard image, or try again later.";
        case "not_found":
            return "The developer image isn't published for this version of AgentMux yet. Pick the standard image, or update AgentMux.";
        default:
            return null;
    }
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
