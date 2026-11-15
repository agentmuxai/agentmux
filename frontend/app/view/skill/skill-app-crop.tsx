// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One reference crop of a computer-use skill's detail (docs/specs/SPEC_PACKS_2026_11_05.md §3.1, §8): the image the
 * agent compares the app's screen with. It is read only when the person asks to see it, so a skill with many crops
 * costs nothing until one is opened, and a changed crop (a different hash) is read again.
 */

// reactive-coverage: one-shot a crop is read when the person opens it and again when its hash changes; the skill list reloads on its own events

import { createResource, createSignal, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { ReferenceInfo } from "@/types/rpc/ReferenceInfo";
import "./skill-app.scss";

export function CropImage(p: { skillId: string; reference: ReferenceInfo }): JSX.Element {
    const [open, setOpen] = createSignal(false);
    const [image] = createResource(
        () => (open() ? p.reference.sha256 : null),
        async (sha) => (sha ? ((await RpcApi.SkillCatalogAppFileCommand(TabRpcClient, { skill_id: p.skillId, path: p.reference.path })).image_data_url ?? "") : ""),
    );
    return (
        <>
            <Button density="compact" aria-expanded={open()} onClick={() => setOpen(!open())}>
                {open() ? "Hide" : "Show"}
            </Button>
            <Show when={open()}>
                <Show when={!image.error} fallback={<p class="skill-app-line skill-app-warn" role="alert">{String(image.error?.message ?? image.error)}</p>}>
                    <Show when={image()} fallback={<p class="skill-app-muted">{image.loading ? "Loading..." : "No image."}</p>}>
                        {(src) => <img class="skill-app-crop" src={src()} alt={`${p.reference.label}: ${p.reference.shows}`} />}
                    </Show>
                </Show>
            </Show>
        </>
    );
}
