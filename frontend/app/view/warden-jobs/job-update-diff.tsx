// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A job's template update (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4): a job keeps the
// version it was made from, and a newer one is offered with a diff of the brief, the outputs and the
// policy. Accepting changes the job exactly as shown. A version the operator hasn't approved can't be
// accepted: they review and approve it first.

import { createSignal, For, onMount, Show, type JSX } from "solid-js";

import { Button, Pill } from "@/app/element/ui";
import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";
import type { JobView } from "@/types/rpc/JobView";
import type { TemplateUpdateDiff } from "@/types/rpc/TemplateUpdateDiff";

import { TemplateApproval } from "./job-template-approval";
import type { JobsModel } from "./jobs-model";

import "./job-editor.scss";

function Change(props: { label: string; added: readonly string[]; removed: readonly string[] }): JSX.Element {
    return (
        <>
            <dt>{props.label}</dt>
            <dd>
                <For each={props.added}>{(a) => <Pill tone="ok">+ {a}</Pill>}</For>
                <For each={props.removed}>{(a) => <Pill tone="error">− {a}</Pill>}</For>
            </dd>
        </>
    );
}

export function JobUpdateDiff(props: { job: JobView; model: JobsModel; onClose: () => void }): JSX.Element {
    const [diff, setDiff] = createSignal<TemplateUpdateDiff | null>(null);
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    const [approving, setApproving] = createSignal<JobTemplateInfo | null>(null);

    const load = async () => {
        try {
            setDiff(await props.model.updateDiff(props.job.id));
            setError(null);
        } catch (e) {
            setError(String(e));
        }
    };
    onMount(() => void load());

    const accept = async () => {
        const d = diff();
        if (!d) return;
        setBusy(true);
        setError(null);
        try {
            const res = await props.model.save({
                job_id: props.job.id,
                definition: { template: { id: d.template_id, version: d.to_version, hash: d.to_hash } },
            });
            if (res.ok) props.onClose();
            else setError(res.error ?? "It wasn't saved.");
        } catch (e) {
            setError(String(e));
        } finally {
            setBusy(false);
        }
    };

    const review = async () => {
        const d = diff();
        if (!d) return;
        try {
            setApproving((await props.model.templateDetail(d.template_id)).template);
        } catch (e) {
            setError(String(e));
        }
    };

    const approve = async () => {
        const info = approving();
        if (!info) return;
        setBusy(true);
        setError(null);
        try {
            await props.model.approveTemplate({
                id: info.id,
                hash: info.hash,
                signer: info.signature.fingerprint ?? "",
            });
            setApproving(null);
            await load();
        } catch (e) {
            setError(String(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <section class="warden-jobs-update" aria-label={`Update for ${props.job.name}`}>
            <Show
                when={approving()}
                fallback={
                    <>
                        <h3 class="warden-jobs-heading">Update for {props.job.name}</h3>
                        <Show
                            when={diff()}
                            fallback={<div class="warden-section-stub">{error() ? "" : "Loading…"}</div>}
                        >
                            {(d) => (
                                <>
                                    <p>
                                        {d().template_id}: <strong>{d().from_version}</strong> →{" "}
                                        <strong>{d().to_version}</strong>
                                    </p>
                                    <Show when={d().status === "none"}>
                                        <div class="warden-section-stub">This job is at the template's version.</div>
                                    </Show>
                                    <Show when={d().status === "unavailable" || d().status === "needs_approval"}>
                                        <div class="warden-jobs-note">{d().reason}</div>
                                    </Show>
                                    <Show when={d().params_error}>
                                        <div class="warden-manager-error" role="alert">
                                            The new version can't take this job's settings as they are:{" "}
                                            {d().params_error}. Edit the job to fix them, then accept.
                                        </div>
                                    </Show>
                                    <Show when={d().status === "available" && !d().params_error}>
                                        <dl class="warden-jobs-facts">
                                            <Show when={d().brief}>
                                                {(b) => (
                                                    <>
                                                        <dt>Brief</dt>
                                                        <dd class="warden-jobs-brief-diff">
                                                            <pre class="warden-jobs-brief" data-side="before">
                                                                {b().before}
                                                            </pre>
                                                            <pre class="warden-jobs-brief" data-side="after">
                                                                {b().after}
                                                            </pre>
                                                        </dd>
                                                    </>
                                                )}
                                            </Show>
                                            <Show when={d().outputs}>
                                                {(o) => (
                                                    <Change label="Outputs" added={o().added} removed={o().removed} />
                                                )}
                                            </Show>
                                            <Show when={d().secrets}>
                                                {(s) => (
                                                    <Change label="Secrets" added={s().added} removed={s().removed} />
                                                )}
                                            </Show>
                                            <For each={d().policy}>
                                                {(c) => (
                                                    <>
                                                        <dt>{c.field}</dt>
                                                        <dd>
                                                            <span class="warden-jobs-before">{c.before}</span> →{" "}
                                                            <span class="warden-jobs-after">{c.after}</span>
                                                        </dd>
                                                    </>
                                                )}
                                            </For>
                                        </dl>
                                        <Show
                                            when={!d().brief && !d().outputs && !d().secrets && d().policy.length === 0}
                                        >
                                            <div class="warden-section-stub">
                                                Nothing the job holds changes; accepting moves it to the new version.
                                            </div>
                                        </Show>
                                    </Show>
                                </>
                            )}
                        </Show>
                        <Show when={error()}>
                            <div class="warden-manager-error" role="alert">
                                {error()}
                            </div>
                        </Show>
                        <div class="warden-jobs-actions">
                            <Show when={diff()?.status === "available" && !diff()?.params_error}>
                                <Button tone="accent" icon="check" busy={busy()} onClick={() => void accept()}>
                                    Accept the update
                                </Button>
                            </Show>
                            <Show when={diff()?.status === "needs_approval"}>
                                <Button tone="accent" icon="shield-halved" onClick={() => void review()}>
                                    Review and approve {diff()?.to_version}
                                </Button>
                            </Show>
                            <Button tone="quiet" onClick={() => props.onClose()}>
                                Close
                            </Button>
                        </div>
                    </>
                }
            >
                {(info) => (
                    <TemplateApproval
                        info={info()}
                        busy={busy()}
                        error={error()}
                        onApprove={() => void approve()}
                        onCancel={() => setApproving(null)}
                    />
                )}
            </Show>
        </section>
    );
}
