// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A job's template update (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4): a job keeps the
// version it was made from, and a newer one is offered with a diff of the brief, the outputs and the
// policy. Accepting changes the job exactly as shown. A version the operator hasn't approved can't be
// accepted: they review and approve it first.

import { Show, type JSX } from "solid-js";

import { TemplateApproval } from "./job-template-approval";
import { UpdateView } from "./job-update-diff-parts";
import { createUpdateDiff, type JobUpdateDiffProps } from "./job-update-diff-state";

import "./job-editor.scss";

export function JobUpdateDiff(props: JobUpdateDiffProps): JSX.Element {
    const s = createUpdateDiff(props);
    return (
        <section class="warden-jobs-update" aria-label={`Update for ${props.job.name}`}>
            <Show when={s.approving()} fallback={<UpdateView s={s} name={props.job.name} onClose={props.onClose} />}>
                {(info) => (
                    <TemplateApproval
                        info={info()}
                        busy={s.busy()}
                        error={s.error()}
                        onApprove={() => void s.approve()}
                        onCancel={s.cancelReview}
                    />
                )}
            </Show>
        </section>
    );
}
