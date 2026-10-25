// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A dry run's answer (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1): the brief as the agent
// would get it, the next fire times in the job's zone, the caps that would apply, the outputs and
// secrets it declares, and whether its runs would wait in a concurrency group. Starting nothing:
// the words say so. A brief is shown as text, never as markup.

import { For, Show, type JSX } from "solid-js";

import { Pill } from "@/app/element/ui";
import type { JobsDryRunResult } from "@/types/rpc/JobsDryRunResult";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

import "./job-editor.scss";

const dollars = (usd: number): string => `$${Number.isInteger(usd) ? usd : usd.toFixed(2)}`;

export function JobDryRun(props: {
    result: JobsDryRunResult;
    /** The Vault's entries when they can be read here, to say which declared secrets have none. */
    vault: readonly VaultEntryInfo[] | null;
    /** An agent's id as its name, for "goes to". */
    nameOf: (agentId: string) => string;
}): JSX.Element {
    const r = () => props.result;
    const missing = (vaultId: string) => props.vault != null && !props.vault.some((e) => e.id === vaultId);
    return (
        <section class="warden-jobs-dryrun" aria-label="Dry run">
            <Show
                when={r().ok}
                fallback={
                    <div class="warden-manager-error" role="alert">
                        Dry run: this wouldn't be saved. {r().error}
                    </div>
                }
            >
                <h3 class="warden-jobs-heading">Dry run: {r().name}</h3>
                <p class="warden-manager-dim">Nothing was started. This is what a run would do.</p>
                <dl class="warden-jobs-facts">
                    <dt>Goes to</dt>
                    <dd>
                        {r().action_kind}
                        <Show when={r().target}> → {props.nameOf(r().target)}</Show>
                    </dd>
                    <dt>Brief</dt>
                    <dd>
                        <pre class="warden-jobs-brief">{r().brief || "—"}</pre>
                    </dd>
                    <dt>Next fires</dt>
                    <dd>
                        <Show
                            when={r().fires.length > 0}
                            fallback={<span class="warden-manager-dim">none: it runs when you run it</span>}
                        >
                            <ul class="warden-jobs-list">
                                <For each={r().fires}>
                                    {(f) => <li title={new Date(f.at_ms).toISOString()}>{f.local}</li>}
                                </For>
                            </ul>
                        </Show>
                    </dd>
                    <Show when={r().caps}>
                        {(caps) => (
                            <>
                                <dt>Caps</dt>
                                <dd>
                                    {dollars(caps().run_usd)} per run, {dollars(caps().day_usd)} per day,{" "}
                                    {dollars(caps().account_day_usd)} per account per day
                                    <Show when={caps().clamped}>
                                        <Pill tone="warn" title="Only the operator raises a cap">
                                            held to the defaults
                                        </Pill>
                                    </Show>
                                </dd>
                            </>
                        )}
                    </Show>
                    <dt>Outputs</dt>
                    <dd>
                        <Show
                            when={r().outputs.length > 0}
                            fallback={<span class="warden-manager-dim">none declared</span>}
                        >
                            <For each={r().outputs}>{(o) => <Pill>{o}</Pill>}</For>
                        </Show>
                    </dd>
                    <dt>Secrets</dt>
                    <dd>
                        <Show
                            when={r().secrets.length > 0}
                            fallback={<span class="warden-manager-dim">none declared</span>}
                        >
                            <ul class="warden-jobs-list">
                                <For each={r().secrets}>
                                    {(s) => (
                                        <li>
                                            {s.name} <span class="warden-manager-dim">from Vault entry {s.vault}</span>
                                            <Show when={missing(s.vault)}>
                                                <Pill tone="warn">not in the Vault</Pill>
                                            </Show>
                                        </li>
                                    )}
                                </For>
                            </ul>
                            <div class="warden-manager-dim">
                                A run asks for each secret's use when it first needs it, unless you allowed it with
                                Always: make those grants before the first run.
                            </div>
                        </Show>
                    </dd>
                    <Show when={r().group}>
                        <dt>Waits in</dt>
                        <dd>
                            the <strong>{r().group}</strong> group: at most one run of it at a time, across all jobs
                        </dd>
                    </Show>
                </dl>
                <For each={r().notes}>{(n) => <div class="warden-jobs-note">{n}</div>}</For>
            </Show>
        </section>
    );
}
