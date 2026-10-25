// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The operator's approval of a job template: who published it and whether that key is the one pinned,
// when it triggers, what it does, every output and secret it declares and its default caps
// (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4). Approving runs nothing; it lets a job be
// made from exactly this version. Agents can't approve.

import { For, Show, type JSX } from "solid-js";

import { Button, Pill, type PillTone } from "@/app/element/ui";
import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";

import { describeCron } from "./jobs-format";

import "./job-editor.scss";

/** Outputs that write outside AgentMux or to someone else (srv's `OUTSIDE_OUTPUTS`). */
const OUTSIDE = new Set(["pull_request", "comment", "message:others", "deploy"]);

const writesOutside = (output: string): boolean => OUTSIDE.has(output) || OUTSIDE.has(output.split(":")[0]);

/** The publisher's key, in words and in a tone. */
export function publisherNote(info: JobTemplateInfo): { text: string; tone: PillTone } {
    const sig = info.signature;
    if (info.source === "builtin") return { text: "Published by AgentMux and part of this app.", tone: "ok" };
    switch (sig.state) {
        case "signed":
            return { text: `Signed with the key pinned for ${sig.publisher} (${sig.fingerprint}).`, tone: "ok" };
        case "signed_new":
            return { text: `${sig.publisher} is new to you. Approving pins its key ${sig.fingerprint}.`, tone: "warn" };
        case "key_changed":
            return {
                text: `The key differs from the one pinned for ${sig.publisher}: pinned ${sig.pinned ?? "none"}, this one ${sig.fingerprint ?? "unsigned"}.`,
                tone: "error",
            };
        default:
            return { text: `Unsigned. Nothing vouches that ${sig.publisher} wrote it.`, tone: "warn" };
    }
}

function triggerWords(trigger: Record<string, unknown> | null | undefined): string {
    if (!trigger) return "none (run by hand)";
    switch (trigger.kind) {
        case "cron":
            return typeof trigger.cron === "string"
                ? describeCron(trigger.cron, typeof trigger.tz === "string" ? trigger.tz : "UTC")
                : "on a schedule";
        case "every":
            return typeof trigger.seconds === "number"
                ? `every ${Math.round(trigger.seconds / 60)} min`
                : "on an interval";
        case "once":
            return "once";
        case "event":
            return "on an event";
        default:
            return "by hand";
    }
}

function capsWords(policy: Record<string, unknown> | null | undefined): string {
    const caps = (policy?.caps ?? {}) as Record<string, unknown>;
    const parts: string[] = [];
    if (typeof caps.run_usd === "number") parts.push(`$${caps.run_usd} per run`);
    if (typeof caps.day_usd === "number") parts.push(`$${caps.day_usd} per day`);
    if (typeof caps.messages_per_day === "number") parts.push(`${caps.messages_per_day} messages per day`);
    return parts.length > 0 ? parts.join(", ") : "the defaults";
}

export function TemplateApproval(props: {
    info: JobTemplateInfo;
    busy: boolean;
    error: string | null;
    onApprove: () => void;
    onCancel: () => void;
}): JSX.Element {
    const m = () => props.info.manifest;
    const note = () => publisherNote(props.info);
    return (
        <section class="warden-jobs-approval" aria-label={`Approve ${props.info.id}`}>
            <Show
                when={m()}
                fallback={<div class="warden-manager-error">{props.info.error ?? "This template can't be read."}</div>}
            >
                {(manifest) => (
                    <>
                        <h3 class="warden-jobs-heading">
                            Approve {manifest().name} {manifest().version}?
                        </h3>
                        <p class="warden-manager-dim">{manifest().description}</p>
                        <dl class="warden-jobs-facts">
                            <dt>Publisher</dt>
                            <dd>
                                <Pill tone={note().tone}>{props.info.signature.publisher}</Pill>{" "}
                                <span>{note().text}</span>
                            </dd>
                            <dt>Triggers</dt>
                            <dd>{triggerWords(manifest().trigger)}</dd>
                            <dt>Does</dt>
                            <dd>
                                {String((manifest().action as Record<string, unknown> | null)?.kind ?? "message")} an
                                agent with the template's brief
                                <Show when={manifest().readsUntrusted}>
                                    <span class="warden-manager-dim"> · reads text nobody vouches for</span>
                                </Show>
                            </dd>
                            <dt>Outputs</dt>
                            <dd>
                                <Show
                                    when={manifest().outputs.length > 0}
                                    fallback={<span class="warden-manager-dim">none</span>}
                                >
                                    <For each={manifest().outputs}>
                                        {(o) => (
                                            <Pill
                                                tone={writesOutside(o) ? "warn" : "neutral"}
                                                title={writesOutside(o) ? "Writes outside AgentMux" : undefined}
                                            >
                                                {o}
                                            </Pill>
                                        )}
                                    </For>
                                </Show>
                            </dd>
                            <dt>Secrets</dt>
                            <dd>
                                <Show
                                    when={manifest().secrets.length > 0}
                                    fallback={<span class="warden-manager-dim">none</span>}
                                >
                                    <For each={manifest().secrets}>
                                        {(s) => <Pill title={s.description ?? undefined}>{s.name}</Pill>}
                                    </For>
                                </Show>
                            </dd>
                            <dt>Default caps</dt>
                            <dd>{capsWords(manifest().policy)}</dd>
                        </dl>
                        <p class="warden-manager-dim">
                            Approving runs nothing. It lets you make jobs from exactly this version; a changed version
                            asks again.
                        </p>
                    </>
                )}
            </Show>
            <Show when={props.error}>
                <div class="warden-manager-error" role="alert">
                    {props.error}
                </div>
            </Show>
            <div class="warden-jobs-actions">
                <Button tone="accent" icon="check" busy={props.busy} disabled={!m()} onClick={() => props.onApprove()}>
                    Approve
                </Button>
                <Button tone="quiet" disabled={props.busy} onClick={() => props.onCancel()}>
                    Cancel
                </Button>
            </div>
        </section>
    );
}
