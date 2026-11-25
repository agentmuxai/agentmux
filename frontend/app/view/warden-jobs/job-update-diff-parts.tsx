// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The job update diff's views: the heading, the offered diff, the error and the actions, over a state from
// job-update-diff-state.ts.

import { For, Show, type Accessor, type JSX } from "solid-js";

import { Button, Pill } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { TemplateUpdateDiff } from "@/types/rpc/TemplateUpdateDiff";

import type { UpdateDiffState } from "./job-update-diff-state";

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

function AvailableFacts(props: { d: Accessor<TemplateUpdateDiff> }): JSX.Element {
    const d = props.d;
    return (
        <>
            <dl class="warden-jobs-facts">
                <Show when={d().brief}>
                    {(b) => (
                        <>
                            <dt>{t("jobs.updateDiff.brief")}</dt>
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
                    {(o) => <Change label={t("jobs.updateDiff.outputs")} added={o().added} removed={o().removed} />}
                </Show>
                <Show when={d().secrets}>
                    {(s) => <Change label={t("jobs.updateDiff.secrets")} added={s().added} removed={s().removed} />}
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
            <Show when={!d().brief && !d().outputs && !d().secrets && d().policy.length === 0}>
                <div class="warden-section-stub">{t("jobs.updateDiff.nothing")}</div>
            </Show>
        </>
    );
}

function DiffBody(props: { d: Accessor<TemplateUpdateDiff> }): JSX.Element {
    const d = props.d;
    return (
        <>
            <p>
                {d().template_id}: <strong>{d().from_version}</strong> → <strong>{d().to_version}</strong>
            </p>
            <Show when={d().status === "none"}>
                <div class="warden-section-stub">{t("jobs.updateDiff.same")}</div>
            </Show>
            <Show when={d().status === "unavailable" || d().status === "needs_approval"}>
                <div class="warden-jobs-note">{d().reason}</div>
            </Show>
            <Show when={d().params_error}>
                <div class="warden-manager-error" role="alert">
                    {t("jobs.updateDiff.paramsError", { error: d().params_error ?? "" })}
                </div>
            </Show>
            <Show when={d().status === "available" && !d().params_error}>
                <AvailableFacts d={d} />
            </Show>
        </>
    );
}

function Actions(props: { s: UpdateDiffState; onClose: () => void }): JSX.Element {
    const s = props.s;
    return (
        <div class="warden-jobs-actions">
            <Show when={s.diff()?.status === "available" && !s.diff()?.params_error}>
                <Button tone="accent" icon="check" busy={s.busy()} onClick={() => s.accept()}>
                    {t("jobs.updateDiff.accept")}
                </Button>
            </Show>
            <Show when={s.diff()?.status === "needs_approval"}>
                <Button tone="accent" icon="shield-halved" onClick={() => s.review()}>
                    {t("jobs.updateDiff.reviewApprove", { version: s.diff()?.to_version ?? "" })}
                </Button>
            </Show>
            <Button tone="quiet" onClick={() => props.onClose()}>
                {t("jobs.updateDiff.close")}
            </Button>
        </div>
    );
}

export function UpdateView(props: { s: UpdateDiffState; name: string; onClose: () => void }): JSX.Element {
    const s = props.s;
    return (
        <>
            <h3 class="warden-jobs-heading">{t("jobs.updateDiff.title", { name: props.name })}</h3>
            <Show when={s.diff()} fallback={<div class="warden-section-stub">{s.error() ? "" : t("jobs.updateDiff.loading")}</div>}>
                {(d) => <DiffBody d={d} />}
            </Show>
            <Show when={s.error()}>
                <div class="warden-manager-error" role="alert">
                    {s.error()}
                </div>
            </Show>
            <Actions s={s} onClose={props.onClose} />
        </>
    );
}
