// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A dry run's answer (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.1): the brief as the agent
// would get it, the next fire times in the job's zone, the caps that would apply, the outputs and
// secrets it declares, and whether its runs would wait in a concurrency group. Starting nothing:
// the words say so. A brief is shown as text, never as markup.

import { For, Show, type JSX } from "solid-js";

import { Pill } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { Rich } from "@/app/i18n/rich";
import type { JobsDryRunResult } from "@/types/rpc/JobsDryRunResult";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

import "./job-editor.scss";
import { tip } from "@/app/element/tip";

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
        <section class="warden-jobs-dryrun" aria-label={t("jobs.dryRun.aria")}>
            <Show
                when={r().ok}
                fallback={
                    <div class="warden-manager-error" role="alert">
                        {t("jobs.dryRun.wontSave", { error: r().error })}
                    </div>
                }
            >
                <h3 class="warden-jobs-heading">{t("jobs.dryRun.heading", { name: r().name })}</h3>
                <p class="warden-manager-dim">{t("jobs.dryRun.nothingStarted")}</p>
                <dl class="warden-jobs-facts">
                    <dt>{t("jobs.dryRun.goesTo")}</dt>
                    <dd>
                        {r().action_kind}
                        <Show when={r().target}> → {props.nameOf(r().target)}</Show>
                    </dd>
                    <dt>{t("jobs.dryRun.brief")}</dt>
                    <dd>
                        <pre class="warden-jobs-brief">{r().brief || "—"}</pre>
                    </dd>
                    <dt>{t("jobs.dryRun.nextFires")}</dt>
                    <dd>
                        <Show
                            when={r().fires.length > 0}
                            fallback={<span class="warden-manager-dim">{t("jobs.dryRun.noFires")}</span>}
                        >
                            <ul class="warden-jobs-list">
                                <For each={r().fires}>
                                    {(f) => <li {...tip(new Date(f.at_ms).toISOString())}>{f.local}</li>}
                                </For>
                            </ul>
                        </Show>
                    </dd>
                    <Show when={r().caps}>
                        {(caps) => (
                            <>
                                <dt>{t("jobs.dryRun.caps")}</dt>
                                <dd>
                                    {t("jobs.dryRun.capsLine", { run: dollars(caps().run_usd), day: dollars(caps().day_usd), account: dollars(caps().account_day_usd) })}
                                    <Show when={caps().clamped}>
                                        <Pill tone="warn" title={t("jobs.dryRun.clampedTip")}>
                                            {t("jobs.dryRun.clamped")}
                                        </Pill>
                                    </Show>
                                </dd>
                            </>
                        )}
                    </Show>
                    <dt>{t("jobs.dryRun.outputs")}</dt>
                    <dd>
                        <Show
                            when={r().outputs.length > 0}
                            fallback={<span class="warden-manager-dim">{t("jobs.dryRun.noneDeclared")}</span>}
                        >
                            <For each={r().outputs}>{(o) => <Pill>{o}</Pill>}</For>
                        </Show>
                    </dd>
                    <dt>{t("jobs.dryRun.secrets")}</dt>
                    <dd>
                        <Show
                            when={r().secrets.length > 0}
                            fallback={<span class="warden-manager-dim">{t("jobs.dryRun.noneDeclared")}</span>}
                        >
                            <ul class="warden-jobs-list job-vault-secrets">
                                <For each={r().secrets}>
                                    {(s) => (
                                        <li>
                                            {s.name} <span class="warden-manager-dim">{t("jobs.dryRun.fromEntry", { vault: s.vault })}</span>
                                            <Show when={missing(s.vault)}>
                                                <Pill tone="warn">{t("jobs.dryRun.notInVault")}</Pill>
                                            </Show>
                                        </li>
                                    )}
                                </For>
                            </ul>
                            <div class="warden-manager-dim">
                                {t("jobs.dryRun.grantsNote")}
                            </div>
                        </Show>
                    </dd>
                    <Show when={r().group}>
                        <dt>{t("jobs.dryRun.waitsIn")}</dt>
                        <dd>
                            <Rich k="jobs.dryRun.waitsInGroup" params={{ group: r().group }} />
                        </dd>
                    </Show>
                </dl>
                <For each={r().notes}>{(n) => <div class="warden-jobs-note">{n}</div>}</For>
            </Show>
        </section>
    );
}
