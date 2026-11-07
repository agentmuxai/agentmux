// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Memory → Skills, the App skill parts (docs/specs/SPEC_PACKS_2026_11_05.md §8): the All | App skills filter and
 * the detail of a computer-use skill: where it came from, its route, files, tests and reference crops.
 */

import { For, Show, type JSX } from "solid-js";
import { Pill, SegmentedControl } from "@/app/element/ui";
import type { SkillApp } from "@/types/rpc/SkillApp";
import {
    appPlatform,
    appStatus,
    describeSignature,
    fileSize,
    pillTone,
    referenceDrift,
    type AppFilter,
} from "./skill-app-model";
import { ScriptReview } from "./skill-app-scripts";
import "./skill-app.scss";

export function AppFilterRow(p: { value: AppFilter; onChange: (v: AppFilter) => void }): JSX.Element {
    return (
        <SegmentedControl
            density="compact"
            ariaLabel="Show skills"
            value={p.value}
            onChange={p.onChange}
            options={[
                { value: "all", label: "All" },
                { value: "app", label: "App skills" },
            ]}
        />
    );
}

const dash = (s: string) => (s ? s : "—");

/** The detail of a computer-use skill, below the usual skill facts. */
export function SkillAppDetail(p: { skillId: string; app: SkillApp }): JSX.Element {
    const platform = appPlatform();
    const status = () => appStatus(p.app, platform);
    const drift = () => referenceDrift(p.app, platform);
    const signature = () => describeSignature(p.app.signature);
    return (
        <section class="skill-app" aria-label="App skill">
            <h4 class="skill-app-title">
                <Pill tone="accent" icon="desktop">
                    App skill
                </Pill>{" "}
                {p.app.vendor} · version {p.app.version}
            </h4>
            <p class="skill-app-line">
                Published by {p.app.publisher}. Works on {p.app.platforms.join(", ") || "no platform listed"}.
            </p>
            <p class={`skill-app-line${signature().tone === "warn" ? " skill-app-warn" : ""}`}>
                <i class={`fa-solid ${signature().tone === "warn" ? "fa-triangle-exclamation" : "fa-signature"}`} aria-hidden="true" />{" "}
                {signature().text}
            </p>
            <p class="skill-app-line">
                <Pill tone={pillTone(status().tone)}>{status().label}</Pill> on this machine
                ({platform}).
            </p>
            <Show when={p.app.ladder.length > 0}>
                <span class="agent-primitive-modal-field-label">Routes, best first</span>
                <ol class="skill-app-list skill-app-list--ordered" aria-label="Routes">
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
                <p class="skill-app-line">Plug-in: the MCP catalog entry {p.app.mcp}.</p>
            </Show>
            <span class="agent-primitive-modal-field-label">Files beside SKILL.md</span>
            <ul class="skill-app-list" aria-label="Files">
                <For each={p.app.files.filter((f) => !f.image)} fallback={<li>none</li>}>
                    {(f) => (
                        <li>
                            {f.path} <span class="skill-app-muted">{fileSize(f.bytes)}</span>
                            <Show when={f.script}>
                                <ScriptReview skillId={p.skillId} file={f} info={p.app.scripts.find((x) => x.path === f.path)} />
                            </Show>
                        </li>
                    )}
                </For>
            </ul>
            <span class="agent-primitive-modal-field-label">Tests</span>
            <ul class="skill-app-list" aria-label="Tests">
                <For each={p.app.verified} fallback={<li>No test recorded yet.</li>}>
                    {(r) => (
                        <li>
                            <Pill tone={r.result === "pass" ? "ok" : "error"}>{r.result}</Pill> {r.appVersion} on {r.platform},{" "}
                            skill {r.skillVersion}, {r.date}
                            <span class="skill-app-muted"> · {dash(r.route)} · {r.runner}</span>
                        </li>
                    )}
                </For>
            </ul>
            <Show when={p.app.references.length > 0}>
                <span class="agent-primitive-modal-field-label">Reference crops</span>
                <Show when={drift()}>{(d) => <p class="skill-app-line skill-app-warn">{d()}</p>}</Show>
                <ul class="skill-app-list" aria-label="Reference crops">
                    <For each={p.app.references}>
                        {(r) => (
                            <li>
                                {r.label} <span class="skill-app-muted">{r.appVersion} · {r.platform} · {r.theme} · {r.scale}% · {r.date}</span>
                                <div class="skill-app-muted">{r.shows}</div>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
        </section>
    );
}
