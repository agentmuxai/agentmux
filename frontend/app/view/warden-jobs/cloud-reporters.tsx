// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The cloud reporters pinned here (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §4.2): the name and key
// fingerprint of each, and the approved cloud jobs that report as it. Pinning a key lets no event in by itself:
// an event also has to match the key of an approved template. Forgetting a reporter stops its jobs' events.

import { createEffect, createSignal, For, on, Show, type JSX } from "solid-js";

import { Button, Field, TextInput } from "@/app/element/ui";
import type { JobReportersResult } from "@/types/rpc/JobReportersResult";

import type { JobsModel } from "./jobs-model";

export function CloudReporters(props: {
    model: Pick<JobsModel, "reporters" | "addReporter" | "forgetReporter">;
    /** Counts the list's polls: each reads the reporters again, since approving a template pins one. */
    polled: number;
    onClose: () => void;
}): JSX.Element {
    const [result, setResult] = createSignal<JobReportersResult | null>(null);
    const [note, setNote] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal<string | null>(null);
    const [confirm, setConfirm] = createSignal<string | null>(null);
    const [name, setName] = createSignal("");
    const [key, setKey] = createSignal("");

    // A pin or forget changes the list: a poll that began before it ended must not put the old list back.
    let version = 0;
    const load = async () => {
        const began = version;
        try {
            const got = await props.model.reporters();
            if (began === version) setResult(got);
        } catch (e) {
            setNote(String(e));
        }
    };
    createEffect(on(() => props.polled, () => void load()));

    const run = async (id: string, what: () => Promise<JobReportersResult>, done?: () => void) => {
        setBusy(id);
        setNote(null);
        version++;
        try {
            const got = await what();
            version++;
            setResult(got);
            done?.();
        } catch (e) {
            setNote(String(e));
        } finally {
            setBusy(null);
            setConfirm(null);
        }
    };

    const add = () =>
        run("add", () => props.model.addReporter(name().trim(), key().trim()), () => {
            setName("");
            setKey("");
        });

    return (
        <section class="warden-jobs-history warden-jobs-reporters" aria-label="Cloud reporters">
            <header class="warden-jobs-history-head">
                <div class="warden-jobs-history-title">
                    <strong>Cloud reporters</strong>
                    <span class="warden-manager-dim">
                        A pinned key lets no event in by itself: an approved cloud job has to name the same key.
                    </span>
                </div>
                <div class="warden-jobs-actions">
                    <Button density="compact" tone="quiet" icon="xmark" onClick={() => props.onClose()}>
                        Close
                    </Button>
                </div>
            </header>
            <Show when={note()}>
                <div class="warden-manager-error">{note()}</div>
            </Show>
            <Show when={result()} fallback={<div class="warden-section-stub">Loading…</div>}>
                {(r) => (
                    <Show when={r().reporters.length > 0} fallback={<div class="warden-section-stub">No reporter is pinned.</div>}>
                        <table class="warden-manager-table">
                            <thead>
                                <tr>
                                    <th>Reporter</th>
                                    <th>Key</th>
                                    <th>Jobs</th>
                                    <th />
                                </tr>
                            </thead>
                            <tbody>
                                <For each={r().reporters}>
                                    {(rep) => (
                                        <tr>
                                            <td>{rep.name}</td>
                                            <td class="warden-jobs-fingerprint">
                                                {rep.fingerprint}
                                            </td>
                                            <td>
                                                <Show when={rep.templates.length > 0} fallback={<span class="warden-manager-dim">none approved</span>}>
                                                    {rep.templates.join(", ")}
                                                </Show>
                                            </td>
                                            <td class="warden-jobs-links">
                                                <Show
                                                    when={confirm() === rep.name}
                                                    fallback={
                                                        <Button
                                                            density="compact"
                                                            tone="danger"
                                                            icon="trash"
                                                            disabled={busy() != null}
                                                            onClick={() => setConfirm(rep.name)}
                                                        >
                                                            Forget
                                                        </Button>
                                                    }
                                                >
                                                    <Button
                                                        density="compact"
                                                        tone="danger"
                                                        icon="trash"
                                                        busy={busy() === `forget:${rep.name}`}
                                                        title={rep.templates.length > 0 ? "Its approved jobs stop receiving events" : undefined}
                                                        onClick={() => void run(`forget:${rep.name}`, () => props.model.forgetReporter(rep.name))}
                                                    >
                                                        Forget it
                                                    </Button>
                                                    <Button density="compact" tone="quiet" onClick={() => setConfirm(null)}>
                                                        Keep
                                                    </Button>
                                                </Show>
                                            </td>
                                        </tr>
                                    )}
                                </For>
                            </tbody>
                        </table>
                    </Show>
                )}
            </Show>
            <div class="warden-jobs-reporter-add">
                <Field label="Name" layout="stacked" description="Lowercase letters, digits and dashes.">
                    <TextInput value={name()} onInput={(e) => setName(e.currentTarget.value)} spellcheck={false} />
                </Field>
                <Field label="Public key" layout="stacked" description="An Ed25519 public key, base64.">
                    <TextInput value={key()} onInput={(e) => setKey(e.currentTarget.value)} spellcheck={false} />
                </Field>
                <Button
                    density="compact"
                    tone="accent"
                    icon="plus"
                    busy={busy() === "add"}
                    disabled={busy() != null || !name().trim() || !key().trim()}
                    onClick={() => void add()}
                >
                    Pin reporter
                </Button>
            </div>
        </section>
    );
}
