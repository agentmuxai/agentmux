// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Jobs section's editor: New job (blank, or from a template), Edit and Clone
// (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.2, §5.4). It builds a job definition and
// sends it with `jobs.save`, which only the operator's own window may do. Dry run shows what the
// definition would do and starts nothing. A template must be approved before a job is made from it.

import { createMemo, createSignal, For, onMount, Show, type JSX } from "solid-js";
import { createStore, reconcile } from "solid-js/store";

import {
    Button,
    Field,
    Pill,
    SegmentedControl,
    Select,
    TextInput,
    type PillTone,
    type SelectOption,
} from "@/app/element/ui";
import { useAgentList } from "@/app/view/agent/use-agent-list";
import { refreshVault, vaultAvailable, vaultEntries } from "@/app/view/vault/vault-store";
import type { CapDefaults } from "@/types/rpc/CapDefaults";
import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";
import type { JobsDryRunResult } from "@/types/rpc/JobsDryRunResult";
import type { JobsSaveResult } from "@/types/rpc/JobsSaveResult";

import {
    blankDraft,
    buildDefinition,
    draftFromDefinition,
    draftFromTemplate,
    type JobDraft,
    type ParamValue,
} from "./job-draft";
import { JobDryRun } from "./job-dry-run";
import { PolicySection, WhatSection, WhenSection, withCurrent } from "./job-editor-fields";
import { JobParamsForm } from "./job-params-form";
import { TemplateApproval } from "./job-template-approval";
import type { JobsModel } from "./jobs-model";

import "./job-editor.scss";

export type EditorStart = { mode: "new" } | { mode: "edit" | "clone"; jobId: string };

const STATE_PILL: Record<string, { label: string; tone: PillTone }> = {
    approved: { label: "Approved", tone: "ok" },
    needs_approval: { label: "Needs your approval", tone: "warn" },
    changed: { label: "Changed since approved", tone: "warn" },
    invalid: { label: "Can't be read", tone: "error" },
};

export function JobEditor(props: {
    start: EditorStart;
    model: JobsModel;
    defaults: CapDefaults | undefined;
    onClose: () => void;
}): JSX.Element {
    const [draft, setDraft] = createStore<JobDraft>(blankDraft());
    const [templates, setTemplates] = createSignal<JobTemplateInfo[]>([]);
    const [loading, setLoading] = createSignal(props.start.mode !== "new");
    const [source, setSource] = createSignal<"blank" | "template">("blank");
    const [chosen, setChosen] = createSignal(false);
    const [approving, setApproving] = createSignal<JobTemplateInfo | null>(null);
    const [busy, setBusy] = createSignal<"save" | "dry" | "approve" | null>(null);
    const [errors, setErrors] = createSignal<string[]>([]);
    const [paramErrors, setParamErrors] = createSignal<Record<string, string>>({});
    const [failure, setFailure] = createSignal<string | null>(null);
    const [dry, setDry] = createSignal<JobsDryRunResult | null>(null);
    const [confirm, setConfirm] = createSignal<JobsSaveResult | null>(null);
    const [agents] = useAgentList();

    const manifest = createMemo(() => templates().find((t) => t.id === draft.template?.id)?.manifest ?? null);
    const agentOptions = createMemo<SelectOption[]>(() =>
        (agents() ?? []).filter((a) => !a.is_seeded).map((a) => ({ value: a.id, label: a.name }))
    );
    const nameOf = (id: string) => agentOptions().find((o) => o.value === id)?.label ?? id;
    const vault = () => (vaultAvailable() ? (vaultEntries() ?? []) : null);

    onMount(async () => {
        if (vaultAvailable()) void refreshVault();
        try {
            setTemplates((await props.model.templates()).templates);
        } catch (e) {
            setFailure(String(e));
        }
        const start = props.start;
        if (start.mode === "new") return;
        try {
            const { definition } = await props.model.get(start.jobId);
            const id = (definition.template as { id?: string } | undefined)?.id;
            const info = templates().find((t) => t.id === id);
            setDraft(reconcile(draftFromDefinition(definition, info?.manifest ?? null, start.mode)));
            setChosen(true);
        } catch (e) {
            setFailure(String(e));
        } finally {
            setLoading(false);
        }
    });

    const chooseSource = (next: "blank" | "template") => {
        setSource(next);
        setChosen(false);
        setDry(null);
        setDraft(reconcile(blankDraft()));
    };

    const useTemplate = (info: JobTemplateInfo) => {
        setDraft(reconcile(draftFromTemplate(info)));
        setChosen(true);
        setDry(null);
    };

    const approve = async () => {
        const info = approving();
        if (!info) return;
        setBusy("approve");
        setFailure(null);
        try {
            const res = await props.model.approveTemplate({
                id: info.id,
                hash: info.hash,
                signer: info.signature.fingerprint ?? "",
            });
            setTemplates(res.templates);
            setApproving(null);
        } catch (e) {
            setFailure(String(e));
        } finally {
            setBusy(null);
        }
    };

    /** The definition for the draft, or null (with the reasons shown) when it can't be sent yet. */
    const built = () => {
        const b = buildDefinition(draft, manifest());
        setErrors(b.errors);
        setParamErrors(b.paramErrors);
        return b.errors.length > 0 ? null : b.definition;
    };

    const save = async (confirmNoCost = false) => {
        setFailure(null);
        const definition = built();
        if (!definition) return;
        setBusy("save");
        try {
            const res = await props.model.save({
                ...(draft.id ? { job_id: draft.id } : {}),
                definition,
                ...(confirmNoCost ? { confirm_no_cost: true } : {}),
            });
            if (res.needs_confirmation) setConfirm(res);
            else props.onClose();
        } catch (e) {
            setFailure(String(e));
        } finally {
            setBusy(null);
        }
    };

    const dryRun = async () => {
        setFailure(null);
        const definition = built();
        if (!definition) return;
        setBusy("dry");
        try {
            setDry(await props.model.dryRun({ ...(draft.id ? { job_id: draft.id } : {}), definition }));
        } catch (e) {
            setFailure(String(e));
        } finally {
            setBusy(null);
        }
    };

    const title = () =>
        props.start.mode === "edit"
            ? `Edit ${draft.name || "job"}`
            : props.start.mode === "clone"
              ? "Clone job"
              : "New job";
    const showForm = () => !loading() && (props.start.mode !== "new" || source() === "blank" || chosen());

    return (
        <section class="warden-jobs-editor" aria-label={title()}>
            <h3 class="warden-jobs-heading">{title()}</h3>
            <Show when={approving()}>
                {(info) => (
                    <TemplateApproval
                        info={info()}
                        busy={busy() === "approve"}
                        error={failure()}
                        onApprove={() => void approve()}
                        onCancel={() => setApproving(null)}
                    />
                )}
            </Show>
            <Show when={!approving()}>
                <Show when={props.start.mode === "new"}>
                    <Field label="Start from" layout="stacked">
                        <SegmentedControl
                            options={[
                                { value: "blank", label: "A blank job" },
                                { value: "template", label: "A template" },
                            ]}
                            value={source()}
                            onChange={chooseSource}
                        />
                    </Field>
                    <Show when={source() === "template" && !chosen()}>
                        <Show
                            when={templates().length > 0}
                            fallback={<div class="warden-section-stub">No templates are installed.</div>}
                        >
                            <ul class="warden-jobs-templates">
                                <For each={templates()}>
                                    {(t) => {
                                        const pill = STATE_PILL[t.state] ?? STATE_PILL.invalid;
                                        return (
                                            <li class="warden-jobs-template">
                                                <div class="warden-jobs-template-head">
                                                    <strong>{t.manifest?.name ?? t.id}</strong>
                                                    <span class="warden-manager-dim">{t.manifest?.version}</span>
                                                    <Pill tone={pill.tone}>{pill.label}</Pill>
                                                    <Show when={t.source === "builtin"}>
                                                        <Pill>AgentMux</Pill>
                                                    </Show>
                                                </div>
                                                <div class="warden-manager-dim">
                                                    {t.manifest?.description ?? t.error}
                                                </div>
                                                <div class="warden-jobs-actions">
                                                    <Show when={t.state === "approved"}>
                                                        <Button
                                                            density="compact"
                                                            icon="clone"
                                                            onClick={() => useTemplate(t)}
                                                        >
                                                            Clone to a job
                                                        </Button>
                                                    </Show>
                                                    <Show when={t.state === "needs_approval" || t.state === "changed"}>
                                                        <Button
                                                            density="compact"
                                                            icon="shield-halved"
                                                            onClick={() => setApproving(t)}
                                                        >
                                                            Review and approve
                                                        </Button>
                                                    </Show>
                                                </div>
                                            </li>
                                        );
                                    }}
                                </For>
                            </ul>
                        </Show>
                    </Show>
                </Show>
                <Show when={loading()}>
                    <div class="warden-section-stub">Loading…</div>
                </Show>
                <Show when={showForm()}>
                    <Show when={draft.template}>
                        {(t) => (
                            <div class="warden-manager-dim">
                                Made from <strong>{manifest()?.name ?? t().id}</strong>{" "}
                                {t().version || manifest()?.version}. The template writes the brief; its settings are
                                below.
                            </div>
                        )}
                    </Show>
                    <div class="warden-jobs-form">
                        <div class="warden-jobs-grid">
                            <Field label="Name" layout="stacked">
                                <TextInput
                                    value={draft.name}
                                    onInput={(e) => setDraft("name", e.currentTarget.value)}
                                    maxLength={200}
                                />
                            </Field>
                            <Field label="Owner" layout="stacked" description="Its leader gets the failure alerts.">
                                <Select
                                    value={draft.owner}
                                    onChange={(v) => setDraft("owner", v)}
                                    options={withCurrent(agentOptions(), draft.owner, "You (no agent)")}
                                />
                            </Field>
                        </div>
                        <Show when={draft.template && manifest()}>
                            {(m) => (
                                <JobParamsForm
                                    manifest={m()}
                                    values={draft.params}
                                    errors={paramErrors()}
                                    vault={vault()}
                                    onChange={(name: string, value: ParamValue) => setDraft("params", name, value)}
                                />
                            )}
                        </Show>
                        <Show when={draft.template && !manifest() && !loading()}>
                            <div class="warden-manager-error">
                                This job's template isn't installed any more, so its settings stay as they are.
                            </div>
                        </Show>
                        <WhenSection draft={draft} set={setDraft} />
                        <WhatSection
                            draft={draft}
                            set={setDraft}
                            agents={agentOptions()}
                            templated={draft.template != null}
                        />
                        <PolicySection draft={draft} set={setDraft} defaults={props.defaults} />
                    </div>
                </Show>
                <Show when={errors().length > 0}>
                    <ul class="warden-manager-error warden-jobs-problems" role="alert">
                        <For each={errors()}>{(e) => <li>{e}</li>}</For>
                    </ul>
                </Show>
                <Show when={failure()}>
                    <div class="warden-manager-error" role="alert">
                        {failure()}
                    </div>
                </Show>
                <Show when={confirm()}>
                    {(c) => (
                        <div class="warden-jobs-note" role="alert">
                            {c().error}
                            <div class="warden-jobs-actions">
                                <Button
                                    tone="danger"
                                    icon="check"
                                    busy={busy() === "save"}
                                    onClick={() => void save(true)}
                                >
                                    Confirm and save
                                </Button>
                                <Button tone="quiet" onClick={() => setConfirm(null)}>
                                    Not now
                                </Button>
                            </div>
                        </div>
                    )}
                </Show>
                <Show when={dry()}>{(d) => <JobDryRun result={d()} vault={vault()} nameOf={nameOf} />}</Show>
                <div class="warden-jobs-actions">
                    <Button
                        tone="accent"
                        icon="floppy-disk"
                        busy={busy() === "save"}
                        disabled={!showForm() || busy() != null}
                        onClick={() => void save()}
                    >
                        {draft.id ? "Save changes" : "Create job"}
                    </Button>
                    <Button
                        icon="vial"
                        busy={busy() === "dry"}
                        disabled={!showForm() || busy() != null}
                        onClick={() => void dryRun()}
                    >
                        Dry run
                    </Button>
                    <Button tone="quiet" onClick={() => props.onClose()}>
                        Cancel
                    </Button>
                </div>
            </Show>
        </section>
    );
}
