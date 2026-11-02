// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What a cloud job adds to the Jobs tab (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.3): its reporter and
// timing in a history's title, and "Run now" as a link or a note, since it runs elsewhere and AgentMux starts nothing.

import { Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { openLink } from "@/app/store/global";
import type { CloudJobView } from "@/types/rpc/CloudJobView";

/** "reports as x · 30 min grace · lost after 120 min". */
export function CloudFacts(props: { cloud: CloudJobView }): JSX.Element {
    const c = () => props.cloud;
    return (
        <span class="warden-manager-dim">
            reports as {c().reporter}
            {c().grace_minutes != null ? ` · ${c().grace_minutes} min grace` : ""}
            {c().max_run_minutes != null ? ` · lost after ${c().max_run_minutes} min` : ""}
        </span>
    );
}

/** Run now for a cloud job: the manifest's page (https only), else what to ask its owner. */
export function CloudRunNow(props: { cloud: CloudJobView }): JSX.Element {
    return (
        <Show
            when={props.cloud.run_now_link}
            fallback={
                <Show when={props.cloud.run_now_ask_owner}>
                    {(ask) => <span class="warden-manager-dim">To run it now: {ask()} (it runs elsewhere, so AgentMux starts nothing)</span>}
                </Show>
            }
        >
            {(url) => (
                <Button
                    density="compact"
                    icon="arrow-up-right-from-square"
                    title={`Opens ${url()}: the job runs elsewhere, so AgentMux starts nothing`}
                    disabled={!url().startsWith("https://")}
                    onClick={() => void openLink(url())}
                >
                    Run now
                </Button>
            )}
        </Show>
    );
}
