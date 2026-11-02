// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The new-agent draft's runtime (Native or Container): what can run, and the default once the probes land. The
// panel owns it, not the form, because the Tools and Install steps before the form are host steps: a container
// agent's CLI lives in its container (SPEC_CONTAINER_AGENTS_AT_PARITY_2026_10_27.md P10), so those steps wait for
// the runtime and are skipped for a container.

import { createEffect, createMemo, createSignal, onCleanup, onMount, type Accessor } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getCapability, watchCapability } from "@/app/store/toolchain-capabilities";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { getCliCatalogEntry } from "../defaults/cli-catalog";
import { shouldPreselectContainer, type ContainerImageState } from "../defaults/container-default";
import { loadLastRuntime, storeLastRuntime } from "../defaults/runtime-choice";
import type { CreateDraft } from "./AgentCreateForm";

export interface RuntimeChoice {
    containerSupported: Accessor<boolean>;
    canPickContainer: Accessor<boolean>;
    imageAccess: Accessor<ContainerImageState>;
    /** The person chooses: remembered for next time, and the default stops following the probes. */
    pickRuntime: (v: "host" | "container") => void;
    /** The runtime is still being decided: a container is preferred and its probes have not answered. */
    deciding: Accessor<boolean>;
}

export function useRuntimeChoice(
    template: AgentDefinition,
    draft: CreateDraft,
    providerId: Accessor<string>,
): RuntimeChoice {
    const containerSupported = createMemo(() => getCliCatalogEntry(providerId())?.containerSupported ?? true);
    // The shared toolchain-capabilities store (a daemon ping), so this can
    // never disagree with the Toolchain or the launch pre-flight about Docker
    // (#1576; RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md).
    //
    // Docker is re-probed every few seconds and reads "checking" meanwhile: the last answer stands through
    // that, or the runtime would flip to Native and back and the form below would unmount each time.
    const dockerAnswer = createMemo<"available" | "unavailable" | undefined>((last) => {
        const status = getCapability("docker").status;
        return status === "available" || status === "unavailable" ? status : last;
    });
    const dockerUp = () => dockerAnswer() === "available";
    const canPickContainer = () => containerSupported() && dockerUp();
    // Poll while the panel is open so starting Docker Desktop unlocks it.
    onMount(() => onCleanup(watchCapability("docker")));

    // Can the container's image be had? Asked once Docker answers; only a
    // definite refusal counts against the container.
    const [imageAccess, setImageAccess] = createSignal<ContainerImageState>(undefined);
    let imageCheckStarted = false;
    createEffect(() => {
        if (imageCheckStarted || !canPickContainer()) return;
        imageCheckStarted = true;
        void (async () => {
            try {
                const r = await RpcApi.ContainerImageCheckCommand(
                    TabRpcClient,
                    { image: template.container_image ?? "" },
                    { timeout: 10000 },
                );
                setImageAccess(r?.status ?? "unknown");
            } catch {
                setImageAccess("unknown");
            }
        })();
    });

    // What the person chose last time, read once as the panel opens.
    const lastRuntime = loadLastRuntime();
    // Default the runtime once the probes land, never to a mode that can't
    // run, until the user chooses.
    createEffect(() => {
        if (draft.runtimeTouched) return;
        draft.setRuntime(
            shouldPreselectContainer({
                templateType: template.agent_type,
                containerSupported: containerSupported(),
                dockerAvailable: dockerUp(),
                image: imageAccess(),
                lastChoice: lastRuntime,
            })
                ? "container"
                : "host",
        );
    });
    const pickRuntime = (v: "host" | "container") => {
        draft.runtimeTouched = true;
        draft.setRuntime(v);
        // Only a choice the person makes is remembered, not the default the probes picked.
        storeLastRuntime(v);
    };

    // A container is preferred (last choice, or the template's) but Docker or the image check has not answered:
    // the host steps would flash up and then be skipped, so they wait.
    const prefersContainer = lastRuntime === "container" || (lastRuntime !== "host" && template.agent_type === "container");
    const deciding = () => {
        if (draft.runtimeTouched || !prefersContainer || !containerSupported()) return false;
        if (dockerAnswer() === undefined) return true;
        return dockerUp() && imageAccess() === undefined;
    };

    return { containerSupported, canPickContainer, imageAccess, pickRuntime, deciding };
}
