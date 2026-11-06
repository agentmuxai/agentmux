// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// One job opened in the Jobs section: its actions (Run now, Pause / Resume, Delete, Open owner) and
// its runs, newest first, with the slot, start, duration, state and reason, outcome and summary,
// cost and account, and links (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.3), the Kanban
// task of a failing job's incident among them (§3.4). A summary is another agent's words: shown as
// text, never as markup.

import { Show, type JSX } from "solid-js";

import { HistoryActions, HistoryDryRun, HistoryRuns, HistoryTitle } from "./jobs-history-parts";
import { createJobHistory, type JobHistoryProps } from "./jobs-history-state";

export { mergeReload } from "./jobs-history-state";

export function JobHistory(props: JobHistoryProps): JSX.Element {
    const h = createJobHistory(props);
    return (
        <section class="warden-jobs-history" aria-label={`Runs of ${props.job.name}`}>
            <header class="warden-jobs-history-head">
                <HistoryTitle props={props} />
                <HistoryActions props={props} h={h} />
            </header>
            <Show when={h.note()}>
                <div class="warden-manager-error">{h.note()}</div>
            </Show>
            <HistoryDryRun h={h} />
            <HistoryRuns props={props} h={h} />
        </section>
    );
}
