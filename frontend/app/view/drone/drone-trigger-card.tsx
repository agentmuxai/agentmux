// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Trigger card: what starts this drone, a strip above the canvas (so it never covers a node). It is a view, not
// a node of the graph and not a second scheduler: the jobs that run the drone own their triggers (real-world jobs
// spec §3.1), so the card lists them and opens the Jobs pane; "By hand" is the run the bar's Run button also makes.

import { For, Show, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { tip } from "@/app/element/tip";
import { openWarden } from "@/app/view/warden/open-warden";
import type { JobPayload } from "@/types/rpc/JobPayload";
import type { DroneViewModel } from "./drone-model";
import { DroneFireAgents } from "./drone-fire-agents";

/** What a job's trigger kind reads as; a kind this build does not know shows as it is named. */
export function triggerKindLabel(kind: string): string {
    switch (kind) {
        case "timer":
            return t("drone.trigger.kindTimer");
        case "app_event":
            return t("drone.trigger.kindAppEvent");
        case "file_watch":
            return t("drone.trigger.kindFileWatch");
        case "webhook_local":
            return t("drone.trigger.kindWebhook");
        case "poll":
            return t("drone.trigger.kindPoll");
        case "expect":
            return t("drone.trigger.kindExpect");
        default:
            return kind;
    }
}

export const TriggerCard = (p: { model: DroneViewModel }): JSX.Element => {
    const m = p.model;
    const jobs = (): JobPayload[] => m.payloadSchemaAtom()?.jobs ?? [];
    const canRun = () => m.validate().ok && !m.runningAtom();
    return (
        <section class="drone-trigger" style={{ zoom: m.zoom() }} aria-label={t("drone.trigger.title")}>
            <h3 class="drone-trigger-title">{t("drone.trigger.title")}</h3>
            <div class="drone-trigger-row">
                <span class="drone-trigger-name">{t("drone.trigger.byHand")}</span>
                <Button class="drone-btn" disabled={!canRun()} onClick={() => m.run()}>
                    {m.runningAtom() ? t("drone.trigger.running") : t("drone.trigger.run")}
                </Button>
            </div>
            <For each={jobs()}>
                {(job) => (
                    <div class="drone-trigger-row">
                        <span class="drone-trigger-name" {...tip(job.job_name)}>
                            {job.job_name}
                        </span>
                        <span class="drone-trigger-kind">{triggerKindLabel(job.trigger)}</span>
                        <Button class="drone-btn" onClick={() => void openWarden("jobs")}>
                            {t("drone.trigger.openJob")}
                        </Button>
                    </div>
                )}
            </For>
            <Show when={m.draftAtom().id}>
                <DroneFireAgents droneId={() => m.draftAtom().id} />
            </Show>
            <Show when={m.draftAtom().id && jobs().length === 0}>
                <p class="drone-trigger-note">{t("drone.trigger.noJobs")}</p>
            </Show>
        </section>
    );
};
