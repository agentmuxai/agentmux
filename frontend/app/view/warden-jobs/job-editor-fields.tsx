// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The editor's sections for a job: when it runs (trigger), what it does (action), how (policy) and
// what it may spend (caps), docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1, §5.3. Each field
// is a control from element/ui/. The caps show the defaults beside them; raising one above its
// default is allowed here, since this is the operator's window (§7).

import { Show, type JSX } from "solid-js";
import type { SetStoreFunction } from "solid-js/store";

import { Field, SegmentedControl, Select, Switch, TextInput, type SelectOption } from "@/app/element/ui";
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

const TRIGGERS: { value: TriggerKind; label: string }[] = [
    { value: "cron", label: "On a schedule" },
    { value: "every", label: "Every" },
    { value: "once", label: "Once" },
    { value: "manual", label: "By hand" },
];

const ACTIONS: { value: ActionKind; label: string }[] = [
    { value: "message", label: "Message an agent" },
    { value: "task", label: "File a task" },
    { value: "spawn", label: "Start a worker" },
];

export function WhenSection(props: SectionProps): JSX.Element {
    return (
        <>
            <Field label="Runs" layout="stacked">
                <SegmentedControl
                    options={TRIGGERS}
                    value={props.draft.triggerKind}
                    onChange={(v) => props.set("triggerKind", v)}
                />
            </Field>
            <Show when={props.draft.triggerKind === "cron"}>
                <div class="warden-jobs-grid">
                    <Field
                        label="Cron expression"
                        layout="stacked"
                        hint={describeCron(props.draft.cron, props.draft.tz || "UTC") || undefined}
                        description="Minute, hour, day of month, month, weekday."
                    >
                        <TextInput
                            value={props.draft.cron}
                            onInput={(e) => props.set("cron", e.currentTarget.value)}
                            spellcheck={false}
                        />
                    </Field>
                    <Field label="Time zone" layout="stacked" description="An IANA name such as Europe/Berlin.">
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
                    label="Every (minutes)"
                    layout="stacked"
                    description="An agent job runs at most every 15 minutes."
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
                <Field label="At" layout="stacked" description="In your time zone.">
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

export function WhatSection(
    props: SectionProps & {
        agents: readonly SelectOption[];
        /** A template writes the brief. */
        templated: boolean;
    }
): JSX.Element {
    const brief = (field: "brief" | "payload", label: string) => (
        <Show
            when={!props.templated}
            fallback={
                <Field label={label} layout="stacked">
                    <div class="warden-manager-dim">Written by the template from its settings. A dry run shows it.</div>
                </Field>
            }
        >
            <Field
                label={label}
                layout="stacked"
                description="Another agent's words to it: it is not your instruction."
            >
                <textarea
                    class="ui-input warden-jobs-textarea"
                    rows={5}
                    aria-label={label}
                    value={props.draft[field]}
                    onInput={(e) => props.set(field, e.currentTarget.value)}
                />
            </Field>
        </Show>
    );
    return (
        <>
            <Field label="Does" layout="stacked">
                <SegmentedControl
                    options={ACTIONS}
                    value={props.draft.actionKind}
                    onChange={(v) => props.set("actionKind", v)}
                />
            </Field>
            <Show when={props.draft.actionKind === "message"}>
                <Field label="Message" layout="stacked">
                    <Select
                        value={props.draft.to}
                        onChange={(v) => props.set("to", v)}
                        options={withCurrent(props.agents, props.draft.to, "Choose an agent")}
                    />
                </Field>
                {brief("brief", "Brief")}
            </Show>
            <Show when={props.draft.actionKind === "task"}>
                <div class="warden-jobs-grid">
                    <Field label="Task title" layout="stacked">
                        <TextInput
                            value={props.draft.title}
                            onInput={(e) => props.set("title", e.currentTarget.value)}
                        />
                    </Field>
                    <Field label="For" layout="stacked">
                        <Select
                            value={props.draft.taskTarget}
                            onChange={(v) => props.set("taskTarget", v)}
                            options={withCurrent(props.agents, props.draft.taskTarget, "Anyone (the backlog)")}
                        />
                    </Field>
                </div>
                {brief("payload", "Details")}
            </Show>
            <Show when={props.draft.actionKind === "spawn"}>
                <Field
                    label="Leader"
                    layout="stacked"
                    description="The worker reports to this agent. A worker starts for each run and spends until it stops."
                >
                    <Select
                        value={props.draft.leader}
                        onChange={(v) => props.set("leader", v)}
                        options={withCurrent(props.agents, props.draft.leader, "Choose an agent")}
                    />
                </Field>
                {brief("brief", "Brief")}
                <div class="warden-jobs-grid">
                    <Field label="Model" layout="stacked" description="Optional.">
                        <TextInput
                            value={props.draft.model}
                            onInput={(e) => props.set("model", e.currentTarget.value)}
                        />
                    </Field>
                    <Field label="Account" layout="stacked" description="Optional.">
                        <TextInput
                            value={props.draft.account}
                            onInput={(e) => props.set("account", e.currentTarget.value)}
                        />
                    </Field>
                </div>
                <Field label="Keep the worker after its first turn" layout="inline">
                    <Switch checked={props.draft.keepWorker} onChange={(on) => props.set("keepWorker", on)} />
                </Field>
            </Show>
        </>
    );
}

export function PolicySection(props: SectionProps & { defaults: CapDefaults | undefined }): JSX.Element {
    const raised = (text: string, base: number | undefined) =>
        base != null && text.trim() !== "" && Number(text) > base;
    const capHint = (text: string, base: number | undefined, what: string) =>
        base == null
            ? undefined
            : raised(text, base)
              ? `Above the default of $${base}: you raise it for this job.`
              : `Default ${what}: $${base}.`;
    return (
        <>
            <div class="warden-jobs-grid">
                <Field label="If the last run is still going" layout="stacked">
                    <Select
                        value={props.draft.overlap}
                        onChange={(v) => props.set("overlap", v)}
                        options={OVERLAPS.map((o) => ({ value: o, label: o.replace("_", " ") }))}
                    />
                </Field>
                <Field
                    label="If a run was missed"
                    layout="stacked"
                    description="Skip it, or run the latest one when the agent is back."
                >
                    <Select
                        value={props.draft.catchUp}
                        onChange={(v) => props.set("catchUp", v)}
                        options={CATCH_UPS.map((o) => ({ value: o, label: o }))}
                    />
                </Field>
                <Field label="Timeout (minutes)" layout="stacked">
                    <TextInput
                        type="number"
                        min="1"
                        value={props.draft.timeoutMinutes}
                        onInput={(e) => props.set("timeoutMinutes", e.currentTarget.value)}
                    />
                </Field>
                <Field label="Retries" layout="stacked" description="Infrastructure failures only, at most 3.">
                    <TextInput
                        type="number"
                        min="0"
                        max="3"
                        value={props.draft.retries}
                        onInput={(e) => props.set("retries", e.currentTarget.value)}
                    />
                </Field>
                <Field label="Group" layout="stacked" description="Runs of one group go one at a time.">
                    <TextInput value={props.draft.group} onInput={(e) => props.set("group", e.currentTarget.value)} />
                </Field>
            </div>
            <div class="warden-jobs-grid">
                <Field
                    label="Cap per run ($)"
                    layout="stacked"
                    hint={capHint(props.draft.runCap, props.defaults?.run_usd, "per run")}
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
                    label="Cap per day ($)"
                    layout="stacked"
                    hint={capHint(props.draft.dayCap, props.defaults?.job_day_usd, "per day")}
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
                    label="Messages per day"
                    layout="stacked"
                    description="Messages to another agent."
                    hint={props.defaults ? `Default ${props.defaults.messages_per_day}.` : undefined}
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
            <Field label="Enabled" layout="inline" description="A disabled job doesn't run on its trigger.">
                <Switch checked={props.draft.enabled} onChange={(on) => props.set("enabled", on)} />
            </Field>
        </>
    );
}
