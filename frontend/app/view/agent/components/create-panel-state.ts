// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The create panel's state: the provider that will run and the runtime choice, decided before anything else
 * because the two host steps only apply to a Native agent.
 */

import { createResource } from "solid-js";

import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import { getProvider } from "../providers";
import type { AgentCreatePanelProps } from "./create-panel-model";
import { useRuntimeChoice } from "./use-runtime-choice";

export function createPanelState(props: AgentCreatePanelProps) {
    // The provider that will run, through the launch's own resolver (#2594).
    const [providerId] = createResource(() => props.template, resolveEffectiveLaunchProvider);
    const provider = () => getProvider(providerId() ?? props.template.provider);
    const name = () => getCliCatalogEntry(providerId() ?? props.template.provider)?.displayName ?? props.template.name;
    /** AgentMux doesn't install this CLI: the user installs it with its own installer. */
    const userInstalled = () => !!provider() && !provider()!.npmPackage && !provider()!.managedInstall;

    // Native or Container, decided before the form: the two host steps below only apply to Native.
    const runtime = useRuntimeChoice(props.template, props.draft, () => providerId() ?? props.template.provider);
    const hostSteps = () => props.draft.runtime() === "host";

    return { props, providerId, provider, name, userInstalled, runtime, hostSteps };
}

export type PanelState = ReturnType<typeof createPanelState>;
