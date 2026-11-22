// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Drone template in the Jobs pane's New job list: it makes a drone, not a job. Its name and settings are asked for
// here, in the list item; the new drone opens in the Drone pane. Making a drone starts nothing.

import { createSignal, Show, type JSX } from "solid-js";

import { guarded, makeDroneFrom, TemplateForm } from "@/app/view/drone/drone-template-picker";
import { openDroneGraph } from "@/app/view/drone/open-drone";
import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";

import { paramForm, type ParamValue } from "./job-draft";

/** The name and settings of an approved Drone template, and the button that makes the drone and opens it. */
export const MakeDroneForm = (p: { info: JobTemplateInfo; onDone: () => void; onBack: () => void }): JSX.Element => {
    const [name, setName] = createSignal(p.info.manifest?.name ?? p.info.id);
    const [values, setValues] = createSignal<Record<string, ParamValue>>(
        p.info.manifest ? paramForm(p.info.manifest) : {}
    );
    const [errors, setErrors] = createSignal<Record<string, string>>({});
    const [busy, setBusy] = createSignal(false);
    const [failure, setFailure] = createSignal<string | null>(null);

    const make = () =>
        guarded(setBusy, setFailure, async () => {
            const made = await makeDroneFrom(p.info, name(), values());
            if ("problems" in made) {
                setErrors(made.problems);
                return;
            }
            await openDroneGraph(made.droneId);
            p.onDone();
        });

    return (
        <>
            <Show when={failure()}>
                <div class="warden-manager-error" role="alert">
                    {failure()}
                </div>
            </Show>
            <TemplateForm
                info={p.info}
                name={name()}
                values={values()}
                errors={errors()}
                busy={busy()}
                onName={setName}
                onChange={(key, value) => setValues({ ...values(), [key]: value })}
                onMake={() => void make()}
                onBack={p.onBack}
            />
        </>
    );
};
