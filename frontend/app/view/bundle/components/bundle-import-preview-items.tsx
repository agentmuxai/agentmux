// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The import preview's two item lists: the skills (with their collisions and renames) and the widgets. */

import { For, Show, type JSX } from "solid-js";

import { Button } from "@/element/button";
import { Tag, TextInput } from "@/app/element/ui";
import type { BundleImportPreviewResponse } from "@/app/store/rpc-api";
import { describePermission, describeSignature } from "@/app/view/settings/sections/widget-permissions";
import type { BundleImportPreviewActions, BundleImportPreviewState } from "./bundle-import-preview-state";

interface ItemsProps {
    s: BundleImportPreviewState;
    act: BundleImportPreviewActions;
}

function SkillRow(props: ItemsProps & { skill: BundleImportPreviewResponse["skills"][number] }): JSX.Element {
    const { s, act, skill } = props;
    const row = () => s.skills().find((x) => x.sourceDir === skill.source_dir)!;
    const colliding = skill.collision !== "none";
    const renameConflict = () =>
        colliding &&
        row().renameValue.trim().length > 0 &&
        act.takenSlugsFor(skill.source_dir).has(row().renameValue.trim());
    return (
        <div class="bundle-import-skill-row">
            <label class="bundle-import-checkbox-row">
                <input
                    type="checkbox"
                    checked={row().checked}
                    onChange={(e) => act.updateSkill(skill.source_dir, { checked: e.currentTarget.checked })}
                />
                <Show
                    when={!colliding}
                    fallback={
                        <span class="bundle-import-skill-collision">
                            <Tag size="xs" tone="warn">
                                {skill.collision === "name_conflict"
                                    ? "already exists in your library"
                                    : "another skill in this import uses this name"}
                            </Tag>
                            <span class="bundle-import-item-name-readonly">{skill.slug}</span>
                            <TextInput variant="plain"
                                class="bundle-import-rename-input"
                                placeholder="rename to import (leave blank to skip)"
                                value={row().renameValue}
                                disabled={!row().checked}
                                onInput={(e) => act.updateSkill(skill.source_dir, { renameValue: e.currentTarget.value })}
                            />
                        </span>
                    }
                >
                    <span class="bundle-import-item-name">{skill.slug}</span>
                </Show>
            </label>
            <Show when={skill.description}>
                <p class="bundle-import-item-description">{skill.description}</p>
            </Show>
            <Show when={skill.content_total_chars > 0}>
                <div class="bundle-import-skill-text-block">
                    <Button
                        className="ghost"
                        onClick={() =>
                            s.setSkillTextHidden((prev) => ({
                                ...prev,
                                [skill.source_dir]: !prev[skill.source_dir],
                            }))
                        }
                    >
                        {s.skillTextHidden()[skill.source_dir] ? "Show" : "Hide"} skill text
                    </Button>
                    <Show when={!s.skillTextHidden()[skill.source_dir]}>
                        <pre class="bundle-import-skill-text">{skill.content}</pre>
                    </Show>
                    <Show when={skill.content_truncated}>
                        <div class="bundle-import-hint bundle-import-hint-warn">
                            Not fully shown — {Array.from(skill.content).length} of {skill.content_total_chars}{" "}
                            characters. The whole skill is imported if you tick it.
                        </div>
                    </Show>
                </div>
            </Show>
            <Show when={renameConflict()}>
                <div class="bundle-import-hint bundle-import-hint-warn">
                    This name is also taken — pick another.
                </div>
            </Show>
        </div>
    );
}

export function SkillsSection(props: ItemsProps): JSX.Element {
    return (
        <Show when={props.s.preview.skills.length > 0}>
            <section class="bundle-import-section">
                <h3 class="bundle-import-section-title">Skills</h3>
                <For each={props.s.preview.skills}>
                    {(skill) => <SkillRow s={props.s} act={props.act} skill={skill} />}
                </For>
            </section>
        </Show>
    );
}

function WidgetRow(props: { s: BundleImportPreviewState; w: NonNullable<BundleImportPreviewResponse["widgets"]>[number] }): JSX.Element {
    const { s, w } = props;
    const sig = describeSignature(w.signature);
    return (
        <div class="bundle-import-skill-row" data-widget-id={w.id}>
            <label class="bundle-import-checkbox-row">
                <input
                    type="checkbox"
                    checked={!!s.widgetChecked()[w.id]}
                    disabled={!!w.error || w.same_as_installed}
                    onChange={(e) => s.setWidgetChecked((prev) => ({ ...prev, [w.id]: e.currentTarget.checked }))}
                />
                <span class="bundle-import-item-name">{w.name}</span>
                <span class="bundle-import-item-meta">
                    {w.version}
                    {w.author ? ` · by ${w.author}` : ""}
                </span>
            </label>
            <Show
                when={!w.error}
                fallback={<div class="bundle-import-hint bundle-import-hint-warn">Can't be imported: {w.error}</div>}
            >
                <Show when={w.same_as_installed}>
                    <div class="bundle-import-hint">Already installed at this version.</div>
                </Show>
                <Show when={!w.same_as_installed && w.installed_version}>
                    <div class="bundle-import-hint">Replaces version {w.installed_version} you have.</div>
                </Show>
                <p class={`bundle-import-item-description${sig.warning ? " bundle-import-hint-warn" : ""}`}>{sig.text}</p>
                <p class="bundle-import-item-description">
                    {w.permissions.length > 0
                        ? `It can: ${w.permissions.map((p) => describePermission(p).text).join(" · ")}`
                        : "It asks for no permissions."}
                </p>
            </Show>
        </div>
    );
}

export function WidgetsSection(props: { s: BundleImportPreviewState }): JSX.Element {
    const preview = props.s.preview;
    return (
        <Show when={(preview.widgets ?? []).length > 0}>
            <section class="bundle-import-section">
                <h3 class="bundle-import-section-title">Widgets</h3>
                <div class="bundle-import-hint">Each widget asks for your approval before it runs.</div>
                <For each={preview.widgets}>{(w) => <WidgetRow s={props.s} w={w} />}</For>
            </section>
        </Show>
    );
}
