// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The "Import a skill" panel (Memory → Skills): pick a source, read it, read every skill's whole
 * text (a diff for one that already exists), tick what to bring in. See SkillImportModel.
 */

import { For, Show, type JSX } from "solid-js";
import type { SkillImportItem } from "@/types/rpc/SkillImportItem";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { Button, TextInput } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { ImportAppFacts } from "./skill-app-scripts";
import { diffLineClass, diffLines, lineDiff } from "@/app/view/memory-editor/line-diff";
import { SOURCE_KIND_LABELS, type SkillImportModel, type SkillSourceKind } from "./skill-import-model";

const statusText = (status: string): string =>
    status === "new" || status === "changed" || status === "same" ? t(`skill.import.status.${status}`) : status;

const locationHint = (kind: SkillSourceKind): string => {
    switch (kind) {
        case "folder":
            return t("skill.import.hint.folder");
        case "file":
            return t("skill.import.hint.file");
        case "git":
            return "https://github.com/owner/skills.git";
        case "url":
            return "https://example.com/my-skill/SKILL.md";
    }
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
                    aria-label={t("skill.import.rowAria", { name: item().name })}
                />
                <span class="skill-import-item-name">{item().name}</span>
                <span class="skill-import-item-status">{statusText(item().status)}</span>
            </label>
            <Show when={item().key}>
                <div class="skill-import-item-path">{item().key}</div>
            </Show>
            <Show when={item().description}>
                <div class="skill-import-item-desc">{item().description}</div>
            </Show>
            <Show when={item().app}>{(app) => <ImportAppFacts app={app()} />}</Show>
            <Show when={item().status !== "same"}>
                <Show
                    when={item().status === "changed"}
                    fallback={<pre class="skill-import-text">{item().content || t("skill.import.empty")}</pre>}
                >
                    <div class="skill-import-diff-label">{t("skill.import.diffLabel")}</div>
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
            <h3 class="agent-primitive-modal-name">{t("skill.import.title")}</h3>
            <Show when={model.errorAtom()}>
                <div class="agent-primitive-modal-error">{model.errorAtom()}</div>
            </Show>
            <span class="agent-primitive-modal-field-label">{t("skill.import.from")}</span>
            <select
                class="agent-primitive-modal-input"
                value={model.kindAtom()}
                onChange={(e) => model.setKind(e.currentTarget.value as SkillSourceKind)}
            >
                <For each={Object.keys(SOURCE_KIND_LABELS) as SkillSourceKind[]}>
                    {(kind) => <option value={kind}>{SOURCE_KIND_LABELS[kind]}</option>}
                </For>
            </select>
            <TextInput variant="plain"
                class="agent-primitive-modal-input"
                aria-label={t("skill.import.where")}
                value={model.locationAtom()}
                onInput={(e) => model.setLocation(e.currentTarget.value)}
                onContextMenu={showTextInputContextMenu}
                onKeyDown={(e) => {
                    if (e.key === "Enter") void model.read();
                }}
                placeholder={locationHint(model.kindAtom())}
                spellcheck={false}
            />
            <Show when={model.kindAtom() === "git"}>
                <TextInput variant="plain"
                    class="agent-primitive-modal-input"
                    aria-label={t("skill.import.branch")}
                    value={model.gitRefAtom()}
                    onInput={(e) => model.setGitRef(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                    placeholder={t("skill.import.branchPlaceholder")}
                    spellcheck={false}
                />
            </Show>
            <div class="agent-primitive-modal-actions">
                <Button class="agent-primitive-modal-btn" onClick={() => props.onCancel()}>
                    {t("skill.import.cancel")}
                </Button>
                <Button
                    class="agent-primitive-modal-btn"
                    disabled={model.busyAtom() || !model.locationAtom().trim()}
                    onClick={() => void model.read()}
                >
                    {model.busyAtom() && !model.previewAtom() ? t("skill.import.reading") : t("skill.import.read")}
                </Button>
            </div>
            <SkillImportResult model={model} />
        </div>
    );
}

/** The result of reading a source, shared by the import panel and "check for changes". */
export function SkillImportResult(props: { model: SkillImportModel; verb?: string }): JSX.Element {
    const model = props.model;
    const verb = () => props.verb ?? t("skill.import.verb");
    const count = () =>
        (model.previewAtom()?.items ?? []).filter((i) => model.pickedAtom().has(i.key) && i.status !== "same").length;
    return (
        <Show when={model.previewAtom()}>
            {(preview) => (
                <>
                    <Show when={preview().warnings.length > 0}>
                        <ul class="skill-import-warnings" aria-label={t("skill.import.notes")}>
                            <For each={preview().warnings}>{(w) => <li>{w}</li>}</For>
                        </ul>
                    </Show>
                    <p class="agent-primitive-modal-global-note">
                        {t("skill.import.warning")}
                    </p>
                    <ul class="skill-import-list" aria-label={t("skill.import.found")}>
                        <For each={preview().items}>{(item) => <SkillImportRow model={model} item={item} />}</For>
                    </ul>
                    <div class="agent-primitive-modal-actions">
                        <Button
                            tone="accent"
                            class="agent-primitive-modal-btn agent-primitive-modal-btn-primary"
                            disabled={model.busyAtom() || count() === 0}
                            onClick={() => void model.commit()}
                        >
                            {count() === 0
                                ? t("skill.import.nothing", { verb: verb().toLowerCase() })
                                : t("skill.import.verbCount", { verb: verb(), count: count() })}
                        </Button>
                    </div>
                </>
            )}
        </Show>
    );
}
