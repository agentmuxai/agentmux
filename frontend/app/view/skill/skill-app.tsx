// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Memory → Skills, the App skill parts (docs/specs/SPEC_PACKS_2026_11_05.md §8): the All | App skills filter and
 * the detail of a computer-use skill: where it came from, its route, files, tests and reference crops.
 */

import { createSignal, For, Show, type JSX } from "solid-js";
import { Pill, SegmentedControl } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { AgentDefinition } from "@/types/rpc/AgentDefinition";
import type { AppSkillHere } from "@/types/rpc/AppSkillHere";
import type { SkillApp } from "@/types/rpc/SkillApp";
import {
    appHereFact,
    appPlatform,
    appStatus,
    describeSignature,
    FROM_CATALOG_LINE,
    fileSize,
    osNames,
    pillTone,
    referenceDrift,
    type AppFilter,
} from "./skill-app-model";
import { ExportSkill } from "./skill-app-export";
import { CropImage } from "./skill-app-crop";
import { RunTest } from "./skill-app-run-test";
import { ScriptReview } from "./skill-app-scripts";
import "./skill-app.scss";

export function AppFilterRow(p: { value: AppFilter; onChange: (v: AppFilter) => void }): JSX.Element {
    return (
        <SegmentedControl
            density="compact"
            ariaLabel={t("skill.app.showAria")}
            value={p.value}
            onChange={p.onChange}
            options={[
                { value: "all", label: t("skill.app.filterAll") },
                { value: "app", label: t("skill.app.filterApp") },
            ]}
        />
    );
}

const dash = (s: string) => (s ? s : "—");

/** Who signed the skill, and, when it is exactly what the AgentMux catalog listed, that. */
function SignatureLines(p: { app: SkillApp }): JSX.Element {
    const signature = () => describeSignature(p.app.signature);
    return (
        <>
            <p class={`skill-app-line${signature().tone === "warn" ? " skill-app-warn" : ""}`}>
                <i class={`fa-solid ${signature().tone === "warn" ? "fa-triangle-exclamation" : "fa-signature"}`} aria-hidden="true" />{" "}
                {signature().text}
            </p>
            <Show when={p.app.from_catalog}>
                <p class="skill-app-line">
                    <i class="fa-solid fa-shield-halved" aria-hidden="true" /> {FROM_CATALOG_LINE}
                </p>
            </Show>
        </>
    );
}

/** The application on this computer, and, when it is not the version the last pass was run against, that its test is stale. */
function AppHereLine(p: { app: SkillApp; here: AppSkillHere | undefined; drift: boolean }): JSX.Element {
    const fact = () => appHereFact(p.app, p.here, true);
    return (
        <Show when={fact()}>
            <p class={`skill-app-line${p.drift ? " skill-app-warn" : " skill-app-muted"}`}>
                {fact()}
                {p.drift ? ` ${t("skill.app.drift")}` : ""}
            </p>
        </Show>
    );
}

/** The detail of a computer-use skill, below the usual skill facts. */
export function SkillAppDetail(p: { skillId: string; app: SkillApp; here?: AppSkillHere; agents?: AgentDefinition[] }): JSX.Element {
    const platform = appPlatform();
    const status = () => appStatus(p.app, platform, p.here);
    const drift = () => referenceDrift(p.app, platform, p.here?.found ? p.here.version : null);
    // Which scripts' Review panels are open, kept here because an approval rebuilds the rows below.
    const [reviewing, setReviewing] = createSignal<ReadonlySet<string>>(new Set());
    const toggle = (path: string) =>
        setReviewing((cur) => {
            const next = new Set(cur);
            if (!next.delete(path)) next.add(path);
            return next;
        });
    return (
        <section class="skill-app" aria-label={t("skill.app.aria")}>
            <h4 class="skill-app-title">
                <Pill tone="accent" icon="desktop">
                    {t("skill.app.pill")}
                </Pill>{" "}
                {t("skill.app.vendorVersion", { vendor: p.app.vendor, version: p.app.version })}
            </h4>
            <p class="skill-app-line">
                {t("skill.app.published", { publisher: p.app.publisher, platforms: osNames(p.app.platforms) || t("skill.app.noPlatform") })}
            </p>
            <SignatureLines app={p.app} />
            <p class="skill-app-line">
                <Pill tone={pillTone(status().tone)}>{status().label}</Pill> {t("skill.app.onMachine", { platform })}
            </p>
            <AppHereLine app={p.app} here={p.here} drift={status().tone === "warn"} />
            <RunTest skillId={p.skillId} app={p.app} here={p.here} agents={p.agents ?? []} />
            <Show when={p.app.ladder.length > 0}>
                <span class="agent-primitive-modal-field-label">{t("skill.app.routes")}</span>
                <ol class="skill-app-list skill-app-list--ordered" aria-label={t("skill.app.routesAria")}>
                    <For each={p.app.ladder}>
                        {(step) => (
                            <li>
                                <strong>{step.kind}</strong> {step.note}
                            </li>
                        )}
                    </For>
                </ol>
            </Show>
            <Show when={p.app.mcp}>
                <p class="skill-app-line">{t("skill.app.plugin", { mcp: p.app.mcp })}</p>
            </Show>
            <span class="agent-primitive-modal-field-label">{t("skill.app.files")}</span>
            <ul class="skill-app-list" aria-label={t("skill.app.filesAria")}>
                <For each={p.app.files.filter((f) => !f.image)} fallback={<li>{t("skill.app.none")}</li>}>
                    {(f) => (
                        <li>
                            {f.path} <span class="skill-app-muted">{fileSize(f.bytes)}</span>
                            <Show when={f.script}>
                                <ScriptReview
                                    skillId={p.skillId}
                                    file={f}
                                    info={p.app.scripts.find((x) => x.path === f.path)}
                                    fromCatalog={p.app.from_catalog}
                                    open={reviewing().has(f.path)}
                                    onToggle={() => toggle(f.path)}
                                />
                            </Show>
                        </li>
                    )}
                </For>
            </ul>
            <span class="agent-primitive-modal-field-label">{t("skill.app.tests")}</span>
            <ul class="skill-app-list" aria-label={t("skill.app.testsAria")}>
                <For each={p.app.verified} fallback={<li>{t("skill.app.noTest")}</li>}>
                    {(r) => (
                        <li>
                            <Pill tone={r.result === "pass" ? "ok" : "error"}>{r.result}</Pill>{" "}
                            {t("skill.app.testLine", { app: r.appVersion, platform: r.platform, skill: r.skillVersion, date: r.date })}
                            <span class="skill-app-muted"> · {dash(r.route)} · {r.runner}</span>
                        </li>
                    )}
                </For>
            </ul>
            <Show when={p.app.references.length > 0}>
                <span class="agent-primitive-modal-field-label">{t("skill.app.crops")}</span>
                <Show when={drift()}>{(d) => <p class="skill-app-line skill-app-warn">{d()}</p>}</Show>
                <ul class="skill-app-list" aria-label={t("skill.app.cropsAria")}>
                    <For each={p.app.references}>
                        {(r) => (
                            <li>
                                {r.label} <span class="skill-app-muted">{r.appVersion} · {r.platform} · {r.theme} · {r.scale}% · {r.date}</span>{" "}
                                <CropImage skillId={p.skillId} reference={r} />
                                <div class="skill-app-muted">{r.shows}</div>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
            <ExportSkill skillId={p.skillId} />
        </section>
    );
}
