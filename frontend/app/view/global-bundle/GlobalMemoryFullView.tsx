// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * GlobalMemoryFullView — what a Global Memory tile expands to
 * (docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3/§2.4): one
 * entry (view / edit / history), a new-entry draft, the read-only CLAUDE.md,
 * or the combined preview. Every variant uses the pinned layout — actions,
 * name field, banner, history and hints in the top region; the content or
 * editor filling the bottom. The back/breadcrumb header is the caller's
 * (global-bundle-manager.tsx), same pattern as Personal Memory.
 */

import { createEffect, createMemo, on, onCleanup, Show, untrack, type JSX } from "solid-js";
import { createConfirm, type PaneConfirm } from "@/app/element/create-confirm";
import { Markdown } from "@/app/element/markdown";
import { t } from "@/app/i18n/i18n";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import type { Bundle, GlobalMemoryVersionMeta } from "@/app/store/rpc-api";
import { askDiscard, handleMemoryEditorKeyDown, requestCancel } from "@/app/view/memory-editor/editor-keys";
import { MemoryConflictBanner } from "@/app/view/memory-editor/MemoryConflictBanner";
import { MemoryContent } from "@/app/view/memory-editor/MemoryContent";
import { MemoryHistory } from "@/app/view/memory-editor/MemoryHistory";
import { MemoryHistoryModel } from "@/app/view/memory-editor/memory-history-model";
import { PinnedEditorLayout } from "@/app/view/memory-editor/PinnedEditorLayout";
import {
    globalMemoryHistorySource,
    type GlobalBundleViewModel,
    type GlobalMemoryDraft,
    type GlobalMemoryReach,
} from "./global-bundle-model";
import { keyLabel } from "@/app/keybindings";
import { Button, SegmentedControl, TextInput } from "@/app/element/ui";
import { focusOnOpen } from "@/util/focusutil";
import { tip } from "@/app/element/tip";

const SURFACE = "armory-global";

const reachOptions = (): { value: GlobalMemoryReach; label: string; title: string }[] => [
    { value: "fleet", label: t("globalMemory.full.reach.fleet"), title: t("globalMemory.full.reach.fleetTip") },
    { value: "machine", label: t("globalMemory.full.reach.machine"), title: t("globalMemory.full.reach.machineTip") },
];

const draftText = (v: GlobalMemoryDraft) => `# ${v.name}\n\n${v.instructions}`;

/** Name field + editor bar shared by an existing entry and a new one. */
function EditorBar(props: { model: GlobalBundleViewModel; isNew: boolean; confirm: PaneConfirm }): JSX.Element {
    const draft = props.model.draft;
    const value = () => draft.draftAtom() ?? { name: "", instructions: "" };
    return (
        <>
            <div class="memory-editor-actions">
                <Button
                    density="compact"
                    tone="accent"
                    class="memory-editor-btn is-primary"
                    disabled={draft.savingAtom() || !value().name.trim()}
                    onClick={() => void props.model.save()}
                    title={t("globalMemory.full.saveTip", { key: keyLabel("mod+s") })}
                >
                    {draft.savingAtom() ? t("globalMemory.full.saving") : props.isNew ? t("globalMemory.full.add") : t("globalMemory.full.save")}
                </Button>
                <Button
                    density="compact"
                    class="memory-editor-btn"
                    disabled={draft.savingAtom()}
                    pending={false}
                    onClick={() =>
                        requestCancel({
                            isDirty: draft.dirtyAtom,
                            onCancel: () => (props.isNew ? props.model.close() : draft.cancel()),
                            confirm: askDiscard(props.confirm),
                        })
                    }
                    title={t("globalMemory.full.cancelTip", { key: keyLabel("Escape") })}
                >
                    {t("globalMemory.full.cancel")}
                </Button>
                <span class="memory-editor-status">{draft.dirtyAtom() ? t("globalMemory.full.unsaved") : t("globalMemory.full.noChanges")}</span>
            </div>
            <label class="memory-editor-field">
                <span class="memory-editor-field-label">{t("globalMemory.full.name")}</span>
                <TextInput variant="plain"
                    class="memory-editor-input"
                    value={value().name}
                    onInput={(e) => draft.setDraft({ ...value(), name: e.currentTarget.value })}
                    onContextMenu={showTextInputContextMenu}
                    placeholder={t("globalMemory.full.namePlaceholder")}
                    ref={(el) => {
                        if (props.isNew) focusOnOpen(el);
                    }}
                />
            </label>
        </>
    );
}

/** Asks in the pane whether to remove the entry, and removes it on a yes. The button that calls it waits for a person's
 *  answer, so it is `pending={false}`. */
async function removeAfterConfirm(confirm: PaneConfirm, name: string, remove: () => void): Promise<void> {
    if (!(await confirm.ask({ title: t("globalMemory.full.removeTitle", { name }), confirmLabel: t("globalMemory.full.remove") }))) return;
    remove();
}

/** Mounted once per entry id (not per list refresh — the refreshed Bundle is
 *  a new object every time, and a remount would drop the draft and the open
 *  history selection). If the entry is deleted while open, the last-known
 *  copy stays on screen: a dirty draft gets the "deleted" banner instead of
 *  vanishing with the view. */
function EntryView(props: { model: GlobalBundleViewModel; id: string; confirm: PaneConfirm }): JSX.Element {
    const model = props.model;
    const draft = model.draft;
    const id = props.id;
    const history = new MemoryHistoryModel<GlobalMemoryVersionMeta>(globalMemoryHistorySource(id));
    onCleanup(() => history.dispose());
    // Reload history in place whenever the list refreshes (a save, a revert,
    // an agent's GlobalMemoryWrite arriving via memories:changed).
    createEffect(on(model.refreshNonceAtom, () => void history.loadHistory(), { defer: true }));

    let last: Bundle | null = untrack(model.openEntryAtom);
    const liveEntry = createMemo(() => {
        const e = model.openEntryAtom();
        if (e) last = e;
        return e;
    });
    const entry = (): Bundle => liveEntry() ?? (last as Bundle);
    const isSystem = () => !!entry().is_system;
    const ordIndex = () => model.ordinarySectionsAtom().findIndex((s) => s.id === id);
    // An AgentMux entry never leaves the machine, so it has no switch; neither does an
    // entry before srv has said where it goes.
    const reach = () => (isSystem() ? null : model.reachOf(id));

    const top = (
        <>
            <Show
                when={draft.editingAtom()}
                fallback={
                    <div class="memory-editor-actions">
                        <Button density="compact" class="memory-editor-btn" onClick={() => model.startEdit()}>
                            {t("globalMemory.full.edit")}
                        </Button>
                        {/* ↑/↓ stay here as the keyboard-reachable fallback for the
                            grid's drag-to-reorder. System entries never reorder. */}
                        <Show when={!isSystem()}>
                            <Button
                                density="compact"
                                class="memory-editor-btn"
                                title={t("globalMemory.full.moveEarlier")}
                                disabled={ordIndex() <= 0}
                                onClick={() => void model.move(id, -1)}
                            >
                                ↑
                            </Button>
                            <Button
                                density="compact"
                                class="memory-editor-btn"
                                title={t("globalMemory.full.moveLater")}
                                disabled={ordIndex() === -1 || ordIndex() === model.ordinarySectionsAtom().length - 1}
                                onClick={() => void model.move(id, 1)}
                            >
                                ↓
                            </Button>
                        </Show>
                        <Show when={reach()}>
                            {(current) => (
                                <SegmentedControl
                                    density="compact"
                                    ariaLabel={t("globalMemory.full.reachAria")}
                                    options={reachOptions()}
                                    value={current()}
                                    onChange={(next) => void model.setReach(id, next)}
                                />
                            )}
                        </Show>
                        <span class="memory-editor-actions-spacer" />
                        <Button
                            density="compact"
                            tone="danger"
                            class="memory-editor-btn is-danger"
                            title={t("globalMemory.full.removeThis")}
                            pending={false}
                            onClick={() =>
                                removeAfterConfirm(props.confirm, entry().name, () => void (isSystem() ? model.removeSystem(id) : model.remove(id)))
                            }
                        >
                            {t("globalMemory.full.remove")}
                        </Button>
                    </div>
                }
            >
                <EditorBar model={model} isNew={false} confirm={props.confirm} />
            </Show>
            <Show when={draft.errorAtom()}>
                <div class="memory-editor-error">{draft.errorAtom()}</div>
            </Show>
            <MemoryConflictBanner model={draft} toText={draftText} noun={t("globalMemory.full.noun")} canRecreate={false} />
            <Show when={!liveEntry() && !draft.editingAtom()}>
                <div class="memory-editor-error">{t("globalMemory.full.gone")}</div>
            </Show>
            <Show when={isSystem()}>
                <p class="global-bundle-intro">
                    {t("globalMemory.full.systemNote")}
                </p>
            </Show>
            <MemoryHistory model={history} revertDisabled={draft.dirtyAtom()} />
        </>
    );

    const bottom = (
        <MemoryContent
            content={entry().instructions ?? ""}
            editing={draft.editingAtom()}
            autoFocus
            draft={draft.draftAtom()?.instructions ?? ""}
            onDraftInput={(v) => draft.setDraft({ ...(draft.draftAtom() ?? { name: "", instructions: "" }), instructions: v })}
            emptyText={t("globalMemory.full.empty")}
            textareaLabel={t("globalMemory.full.editName", { name: entry().name })}
            placeholder={
                isSystem()
                    ? t("globalMemory.full.systemPlaceholder")
                    : t("globalMemory.full.placeholder")
            }
        />
    );

    return <PinnedEditorLayout surface={SURFACE} top={top} bottom={bottom} onKeyDown={(e) => keys(e, model, props.confirm)} />;
}

function keys(e: KeyboardEvent, model: GlobalBundleViewModel, confirm: PaneConfirm) {
    const draft = model.draft;
    handleMemoryEditorKeyDown(e, {
        isEditing: draft.editingAtom,
        isDirty: draft.dirtyAtom,
        onSave: () => void model.save(),
        onCancel: () => (model.viewAtom()?.kind === "new" ? model.close() : draft.cancel()),
        confirm: askDiscard(confirm),
    });
}

function NewEntryView(props: { model: GlobalBundleViewModel; confirm: PaneConfirm }): JSX.Element {
    const draft = props.model.draft;
    const top = (
        <>
            <EditorBar model={props.model} isNew={true} confirm={props.confirm} />
            <Show when={draft.errorAtom()}>
                <div class="memory-editor-error">{draft.errorAtom()}</div>
            </Show>
            <p class="global-bundle-intro">{t("globalMemory.full.addedAtEnd")}</p>
        </>
    );
    const bottom = (
        <MemoryContent
            content={null}
            editing={true}
            draft={draft.draftAtom()?.instructions ?? ""}
            onDraftInput={(v) => draft.setDraft({ ...(draft.draftAtom() ?? { name: "", instructions: "" }), instructions: v })}
            textareaLabel={t("globalMemory.full.newContent")}
            placeholder={t("globalMemory.full.placeholder")}
        />
    );
    return <PinnedEditorLayout surface={SURFACE} top={top} bottom={bottom} onKeyDown={(e) => keys(e, props.model, props.confirm)} />;
}

/** Read-only reference display of the CLAUDE.md in the isolated config dir a
 *  default spawned agent launches with (CLAUDE_CONFIG_DIR). NOT part of
 *  AgentMux's own Global Memory composition — see
 *  docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §7. */
function ClaudeConfigView(props: { model: GlobalBundleViewModel }): JSX.Element {
    const cfg = () => props.model.claudeGlobalConfigAtom();
    const top = (
        <>
            <code
                class="global-bundle-file-label"
                {...tip(t("globalMemory.full.claudeTip"))}
            >
                {cfg()?.path}
            </code>
            <p class="global-bundle-intro">
                {t("globalMemory.full.claudeNote")}
            </p>
        </>
    );
    const bottom = (
        <Show
            when={cfg()?.exists}
            fallback={<p class="memory-content-status global-bundle-file-empty">{t("globalMemory.full.noFile")}</p>}
        >
            <MemoryContent content={cfg()?.content ?? ""} editing={false} />
        </Show>
    );
    return <PinnedEditorLayout surface={SURFACE} top={top} bottom={bottom} />;
}

/** The exact block every agent's startup instructions file gets. */
function PreviewView(props: { model: GlobalBundleViewModel }): JSX.Element {
    const top = (
        <p class="global-bundle-intro">
            {t("globalMemory.full.previewNote")}
        </p>
    );
    const bottom = (
        <div class="memory-content">
            <div class="memory-content-markdown">
                <Markdown
                    text={props.model.previewAtom() || t("globalMemory.full.empty")}
                    scrollable={true}
                    contentClass="memory-content-markdown-content"
                />
            </div>
        </div>
    );
    return <PinnedEditorLayout surface={SURFACE} top={top} bottom={bottom} />;
}

export function GlobalMemoryFullView(props: { model: GlobalBundleViewModel }): JSX.Element {
    const confirm = createConfirm();
    // Keyed on kind AND entry id, so opening a different entry remounts —
    // but a list refresh (same view) never does.
    const viewKey = () => {
        const v = props.model.viewAtom();
        if (!v) return null;
        return v.kind === "entry" ? `entry:${v.id}` : v.kind;
    };
    return (
        <>
            <confirm.Dialog />
            <Show when={viewKey()} keyed>
                {(key) => {
                    switch (key) {
                        case "new":
                            return <NewEntryView model={props.model} confirm={confirm} />;
                        case "claude-config":
                            return <ClaudeConfigView model={props.model} />;
                        case "preview":
                            return <PreviewView model={props.model} />;
                        default:
                            return (
                                <Show
                                    when={untrack(props.model.openEntryAtom)}
                                    fallback={<p class="memory-content-status">{t("globalMemory.full.gone")}</p>}
                                >
                                    <EntryView model={props.model} id={key.slice("entry:".length)} confirm={confirm} />
                                </Show>
                            );
                    }
                }}
            </Show>
        </>
    );
}
