// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The "Import a skill" panel (Memory → Skills): pick a source, read it, read every skill's whole
 * text (a diff for one that already exists), tick what to bring in. See SkillImportModel.
 */

import { For, Show, type JSX } from "solid-js";
import type { SkillImportItem } from "@/types/rpc/SkillImportItem";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { Button } from "@/app/element/ui";
import { diffLineClass, diffLines, lineDiff } from "@/app/view/memory-editor/line-diff";
import { SOURCE_KIND_LABELS, type SkillImportModel, type SkillSourceKind } from "./skill-import-model";

const STATUS_TEXT: Record<string, string> = {
    new: "New",
    changed: "Differs from the catalog",
    same: "Already up to date",
};

const LOCATION_HINT: Record<SkillSourceKind, string> = {
    folder: "A folder with a SKILL.md, or a folder of skills",
    git: "https://github.com/owner/skills.git",
    url: "https://example.com/my-skill/SKILL.md",
};

function SkillImportRow(props: { model: SkillImportModel; item: SkillImportItem }): JSX.Element {
    const item = () => props.item;
    const changedText = () =>
        lineDiff(`${item().existing_description}\n\n${item().existing_content}`, `${item().description}\n\n${item().content}`);
    return (
        <li class="skill-import-item" data-status={item().status}>
            <label class="skill-import-item-head">
                <input
                    type="checkbox"
                    checked={props.model.pickedAtom().has(item().key)}
                    disabled={item().status === "same" || props.model.busyAtom()}
                    onChange={() => props.model.toggle(item())}
                    aria-label={`Import ${item().name}`}
                />
                <span class="skill-import-item-name">{item().name}</span>
                <span class="skill-import-item-status">{STATUS_TEXT[item().status] ?? item().status}</span>
            </label>
            <Show when={item().key}>
                <div class="skill-import-item-path">{item().key}</div>
            </Show>
            <Show when={item().description}>
                <div class="skill-import-item-desc">{item().description}</div>
            </Show>
            <Show when={item().status !== "same"}>
                <Show
                    when={item().status === "changed"}
                    fallback={<pre class="skill-import-text">{item().content || "(empty)"}</pre>}
                >
                    <div class="skill-import-diff-label">What would change in the catalog</div>
                    <pre class="skill-import-text" data-testid="skill-import-diff">
                        <For each={diffLines(changedText())}>
                            {(line) => <div class={diffLineClass(line)}>{line || " "}</div>}
                        </For>
                    </pre>
                </Show>
            </Show>
        </li>
    );
}

export function SkillImportPanel(props: { model: SkillImportModel; onCancel: () => void }): JSX.Element {
    const model = props.model;
    return (
        <div class="agent-primitive-modal-form skill-import">
            <h3 class="agent-primitive-modal-name">Import a skill</h3>
            <Show when={model.errorAtom()}>
                <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
            </Show>
            <span class="agent-primitive-modal-field-label">From</span>
            <select
                class="agent-primitive-modal-input"
                value={model.kindAtom()}
                onChange={(e) => model.setKind(e.currentTarget.value as SkillSourceKind)}
            >
                <For each={Object.keys(SOURCE_KIND_LABELS) as SkillSourceKind[]}>
                    {(kind) => <option value={kind}>{SOURCE_KIND_LABELS[kind]}</option>}
                </For>
            </select>
            <input
                class="agent-primitive-modal-input"
                type="text"
                aria-label="Where the skill is"
                value={model.locationAtom()}
                onInput={(e) => model.setLocation(e.currentTarget.value)}
                onContextMenu={showTextInputContextMenu}
                onKeyDown={(e) => {
                    if (e.key === "Enter") void model.read();
                }}
                placeholder={LOCATION_HINT[model.kindAtom()]}
                spellcheck={false}
            />
            <Show when={model.kindAtom() === "git"}>
                <input
                    class="agent-primitive-modal-input"
                    type="text"
                    aria-label="Branch or tag"
                    value={model.gitRefAtom()}
                    onInput={(e) => model.setGitRef(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder="Branch or tag (default branch if empty)"
                    spellcheck={false}
                />
            </Show>
            <div class="agent-primitive-modal-actions">
                <Button class="agent-primitive-modal-btn" onClick={() => props.onCancel()}>
                    Cancel
                </Button>
                <Button
                    class="agent-primitive-modal-btn"
                    disabled={model.busyAtom() || !model.locationAtom().trim()}
                    onClick={() => void model.read()}
                >
                    {model.busyAtom() && !model.previewAtom() ? "Reading…" : "Read"}
                </Button>
            </div>
            <SkillImportResult model={model} />
        </div>
    );
}

/** The result of reading a source, shared by the import panel and "check for changes". */
export function SkillImportResult(props: { model: SkillImportModel; verb?: string }): JSX.Element {
    const model = props.model;
    const verb = () => props.verb ?? "Import";
    const count = () =>
        (model.previewAtom()?.items ?? []).filter((i) => model.pickedAtom().has(i.key) && i.status !== "same").length;
    return (
        <Show when={model.previewAtom()}>
            {(preview) => (
                <>
                    <Show when={preview().warnings.length > 0}>
                        <ul class="skill-import-warnings" aria-label="Notes about the source">
                            <For each={preview().warnings}>{(w) => <li>{w}</li>}</For>
                        </ul>
                    </Show>
                    <p class="agent-primitive-modal-global-note">
                        An imported skill tells every agent that gets it what to do. Read it first. A new one starts
                        bound to nobody and is not given to new agents until you choose that.
                    </p>
                    <ul class="skill-import-list" aria-label="Skills found">
                        <For each={preview().items}>{(item) => <SkillImportRow model={model} item={item} />}</For>
                    </ul>
                    <div class="agent-primitive-modal-actions">
                        <Button
                            tone="accent"
                            class="agent-primitive-modal-btn agent-primitive-modal-btn-primary"
                            disabled={model.busyAtom() || count() === 0}
                            onClick={() => void model.commit()}
                        >
                            {count() === 0 ? `Nothing to ${verb().toLowerCase()}` : `${verb()} ${count()} ${count() === 1 ? "skill" : "skills"}`}
                        </Button>
                    </div>
                </>
            )}
        </Show>
    );
}
