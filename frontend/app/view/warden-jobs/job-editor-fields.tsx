// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The editor's sections for a job: when it runs (trigger), what it does (action), how (policy) and
// what it may spend (caps), docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1, §5.3. Each field
// is a control from element/ui/. The caps show the defaults beside them; raising one above its
// default is allowed here, since this is the operator's window (§7).

import { Show, type JSX } from "solid-js";
import type { SetStoreFunction } from "solid-js/store";

import { t } from "@/app/i18n/i18n";
import { Field, SegmentedControl, Select, Switch, TextArea, TextInput, type SelectOption } from "@/app/element/ui";
import type { CapDefaults } from "@/types/rpc/CapDefaults";

import { CATCH_UPS, OVERLAPS, type ActionKind, type JobDraft, type TriggerKind } from "./job-draft";
import { describeCron } from "./jobs-format";

/** The agents to choose from, with `current` kept even when it isn't one (a job by name). */
export function withCurrent(options: readonly SelectOption[], current: string, blank?: string): SelectOption[] {
    const out = blank != null ? [{ value: "", label: blank }, ...options] : [...options];
    return current && !out.some((o) => o.value === current) ? [...out, { value: current, label: current }] : out;
}

interface SectionProps {
    draft: JobDraft;
    set: SetStoreFunction<JobDraft>;
}

// Built when drawn, not once at load, so a change of language shows.
const triggers = (): { value: TriggerKind; label: string }[] => [
    { value: "cron", label: t("jobs.fields.trigger.cron") },
    { value: "every", label: t("jobs.fields.trigger.every") },
    { value: "once", label: t("jobs.fields.trigger.once") },
    { value: "manual", label: t("jobs.fields.trigger.manual") },
];

const actions = (): { value: ActionKind; label: string }[] => [
    { value: "message", label: t("jobs.fields.action.message") },
    { value: "task", label: t("jobs.fields.action.task") },
    { value: "spawn", label: t("jobs.fields.action.spawn") },
    { value: "drone", label: t("jobs.fields.action.drone") },
];

export function WhenSection(props: SectionProps): JSX.Element {
    return (
        <>
            <Field label={t("jobs.fields.runs")} layout="stacked">
                <SegmentedControl
                    options={triggers()}
                    value={props.draft.triggerKind}
                    onChange={(v) => props.set("triggerKind", v)}
                />
            </Field>
            <Show when={props.draft.triggerKind === "cron"}>
                <div class="warden-jobs-grid">
                    <Field
                        label={t("jobs.fields.cron")}
                        layout="stacked"
                        hint={describeCron(props.draft.cron, props.draft.tz || "UTC") || undefined}
                        description={t("jobs.fields.cronHelp")}
                    >
                        <TextInput
                            value={props.draft.cron}
                            onInput={(e) => props.set("cron", e.currentTarget.value)}
                            spellcheck={false}
                        />
                    </Field>
                    <Field label={t("jobs.fields.timeZone")} layout="stacked" description={t("jobs.fields.timeZoneHelp")}>
                        <TextInput
                            value={props.draft.tz}
                            onInput={(e) => props.set("tz", e.currentTarget.value)}
                            spellcheck={false}
                        />
                    </Field>
                </div>
            </Show>
            <Show when={props.draft.triggerKind === "every"}>
                <Field
                    label={t("jobs.fields.everyMinutes")}
                    layout="stacked"
                    description={t("jobs.fields.everyHelp")}
                >
                    <TextInput
                        type="number"
                        min="1"
                        value={props.draft.everyMinutes}
                        onInput={(e) => props.set("everyMinutes", e.currentTarget.value)}
                    />
                </Field>
            </Show>
            <Show when={props.draft.triggerKind === "once"}>
                <Field label={t("jobs.fields.at")} layout="stacked" description={t("jobs.fields.atHelp")}>
                    <TextInput
                        type="datetime-local"
                        value={props.draft.onceAt}
                        onInput={(e) => props.set("onceAt", e.currentTarget.value)}
                    />
                </Field>
            </Show>
        </>
    );
}

/** The drone a job runs, chosen from the drones this workspace has. */
function DroneField(props: SectionProps & { drones: readonly SelectOption[] }): JSX.Element {
    return (
        <Field
            label={t("jobs.fields.drone")}
            layout="stacked"
            description={t("jobs.fields.droneHint")}
        >
            <Select
                value={props.draft.droneId}
                onChange={(v) => props.set("droneId", v)}
                options={withCurrent(props.drones, props.draft.droneId, "Choose a drone")}
            />
        </Field>
    );
}

/** Timeout, and retries for an agent run: a drone is not run again after a failure. */
function RunLimitFields(props: SectionProps): JSX.Element {
    return (
        <>
            <Field label={t("jobs.fields.timeout")} layout="stacked">
                <TextInput
                    type="number"
                    min="1"
                    value={props.draft.timeoutMinutes}
                    onInput={(e) => props.set("timeoutMinutes", e.currentTarget.value)}
                />
            </Field>
            <Show when={props.draft.actionKind !== "drone"}>
                <Field label={t("jobs.fields.retries")} layout="stacked" description={t("jobs.fields.retriesHelp")}>
                    <TextInput
                        type="number"
                        min="0"
                        max="3"
                        value={props.draft.retries}
                        onInput={(e) => props.set("retries", e.currentTarget.value)}
                    />
                </Field>
            </Show>
        </>
    );
}

type WhatProps = SectionProps & {
    agents: readonly SelectOption[];
    drones: readonly SelectOption[];
    /** A template writes the brief. */
    templated: boolean;
};

export function WhatSection(props: WhatProps): JSX.Element {
    const brief = (field: "brief" | "payload", label: string) => (
        <Show
            when={!props.templated}
            fallback={
                <Field label={label} layout="stacked">
                    <div class="warden-manager-dim">{t("jobs.fields.templated")}</div>
                </Field>
            }
        >
            <Field
                label={label}
                layout="stacked"
                description={t("jobs.fields.briefHelp")}
            >
                <TextArea
                    variant="plain"
                    class="ui-input warden-jobs-textarea"
                    rows={5} aria-label={label}
                    value={props.draft[field]}
                    onInput={(e) => props.set(field, e.currentTarget.value)}
                />
            </Field>
        </Show>
    );
    return (
        <>
            <Field label={t("jobs.fields.does")} layout="stacked">
                <SegmentedControl
                    options={actions()}
                    value={props.draft.actionKind}
                    onChange={(v) => props.set("actionKind", v)}
                />
            </Field>
            <Show when={props.draft.actionKind === "message"}>
                <Field label={t("jobs.fields.message")} layout="stacked">
                    <Select
                        value={props.draft.to}
                        onChange={(v) => props.set("to", v)}
                        options={withCurrent(props.agents, props.draft.to, t("jobs.fields.chooseAgent"))}
                    />
                </Field>
                {brief("brief", t("jobs.fields.brief"))}
            </Show>
            <Show when={props.draft.actionKind === "task"}>
                <div class="warden-jobs-grid">
                    <Field label={t("jobs.fields.taskTitle")} layout="stacked">
                        <TextInput
                            value={props.draft.title}
                            onInput={(e) => props.set("title", e.currentTarget.value)}
                        />
                    </Field>
                    <Field label={t("jobs.fields.for")} layout="stacked">
                        <Select
                            value={props.draft.taskTarget}
                            onChange={(v) => props.set("taskTarget", v)}
                            options={withCurrent(props.agents, props.draft.taskTarget, t("jobs.fields.anyone"))}
                        />
                    </Field>
                </div>
                {brief("payload", t("jobs.fields.details"))}
            </Show>
            <Show when={props.draft.actionKind === "drone"}>
                <DroneField {...props} />
            </Show>
            <Show when={props.draft.actionKind === "spawn"}>
                <Field
                    label={t("jobs.fields.leader")}
                    layout="stacked"
                    description={t("jobs.fields.leaderHelp")}
                >
                    <Select
                        value={props.draft.leader}
                        onChange={(v) => props.set("leader", v)}
                        options={withCurrent(props.agents, props.draft.leader, t("jobs.fields.chooseAgent"))}
                    />
                </Field>
                {brief("brief", t("jobs.fields.brief"))}
                <div class="warden-jobs-grid">
                    <Field label={t("jobs.fields.model")} layout="stacked" description={t("jobs.fields.optional")}>
                        <TextInput
                            value={props.draft.model}
                            onInput={(e) => props.set("model", e.currentTarget.value)}
                        />
                    </Field>
                    <Field label={t("jobs.fields.account")} layout="stacked" description={t("jobs.fields.optional")}>
                        <TextInput
                            value={props.draft.account}
                            onInput={(e) => props.set("account", e.currentTarget.value)}
                        />
                    </Field>
                </div>
                <Field label={t("jobs.fields.keepWorker")} layout="inline">
                    <Switch checked={props.draft.keepWorker} onChange={(on) => props.set("keepWorker", on)} />
                </Field>
            </Show>
        </>
    );
}

export function PolicySection(props: SectionProps & { defaults: CapDefaults | undefined }): JSX.Element {
    const raised = (text: string, base: number | undefined) =>
        base != null && text.trim() !== "" && Number(text) > base;
    const capHint = (text: string, base: number | undefined, per: "run" | "day") =>
        base == null
            ? undefined
            : raised(text, base)
              ? t("jobs.fields.capRaised", { base })
              : t(per === "run" ? "jobs.fields.capDefaultRun" : "jobs.fields.capDefaultDay", { base });
    return (
        <>
            <div class="warden-jobs-grid">
                <Field label={t("jobs.fields.overlap")} layout="stacked">
                    <Select
                        value={props.draft.overlap}
                        onChange={(v) => props.set("overlap", v)}
                        options={OVERLAPS.map((o) => ({ value: o, label: o.replace("_", " ") }))}
                    />
                </Field>
                <Field
                    label={t("jobs.fields.catchUp")}
                    layout="stacked"
                    description={t("jobs.fields.catchUpHelp")}
                >
                    <Select
                        value={props.draft.catchUp}
                        onChange={(v) => props.set("catchUp", v)}
                        options={CATCH_UPS.map((o) => ({ value: o, label: o }))}
                    />
                </Field>
                <RunLimitFields {...props} />
                <Field label={t("jobs.fields.group")} layout="stacked" description={t("jobs.fields.groupHelp")}>
                    <TextInput value={props.draft.group} onInput={(e) => props.set("group", e.currentTarget.value)} />
                </Field>
            </div>
            <div class="warden-jobs-grid">
                <Field
                    label={t("jobs.fields.capRun")}
                    layout="stacked"
                    hint={capHint(props.draft.runCap, props.defaults?.run_usd, "run")}
                >
                    <TextInput
                        type="number"
                        min="0"
                        step="any"
                        value={props.draft.runCap}
                        placeholder={props.defaults ? String(props.defaults.run_usd) : undefined}
                        onInput={(e) => props.set("runCap", e.currentTarget.value)}
                    />
                </Field>
                <Field
                    label={t("jobs.fields.capDay")}
                    layout="stacked"
                    hint={capHint(props.draft.dayCap, props.defaults?.job_day_usd, "day")}
                >
                    <TextInput
                        type="number"
                        min="0"
                        step="any"
                        value={props.draft.dayCap}
                        placeholder={props.defaults ? String(props.defaults.job_day_usd) : undefined}
                        onInput={(e) => props.set("dayCap", e.currentTarget.value)}
                    />
                </Field>
                <Field
                    label={t("jobs.fields.messagesPerDay")}
                    layout="stacked"
                    description={t("jobs.fields.messagesPerDayHelp")}
                    hint={props.defaults ? t("jobs.fields.messagesDefault", { n: props.defaults.messages_per_day }) : undefined}
                >
                    <TextInput
                        type="number"
                        min="0"
                        value={props.draft.messagesPerDay}
                        placeholder={props.defaults ? String(props.defaults.messages_per_day) : undefined}
                        onInput={(e) => props.set("messagesPerDay", e.currentTarget.value)}
                    />
                </Field>
            </div>
            <Field label={t("jobs.fields.enabled")} layout="inline" description={t("jobs.fields.enabledHelp")}>
                <Switch checked={props.draft.enabled} onChange={(on) => props.set("enabled", on)} />
            </Field>
        </>
    );
}
