// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The markup of Memory → Personal's full view of one memory file, in parts (NativeMemoryFileView.tsx): the top
 * region (the Edit, Save and Cancel buttons, a save's error, the "changed since you started editing" banner and the
 * version history) and the bottom region (the content, or the editor). The state is in native-memory-file-view-state.ts.
 */

import { Show, type JSX } from "solid-js";
import type { PaneConfirm } from "@/app/element/create-confirm";
import { askDiscard, requestCancel } from "@/app/view/memory-editor/editor-keys";
import { MemoryConflictBanner } from "@/app/view/memory-editor/MemoryConflictBanner";
import { MemoryContent } from "@/app/view/memory-editor/MemoryContent";
import { MemoryHistory } from "@/app/view/memory-editor/MemoryHistory";
import { keyLabel } from "@/app/keybindings";
import { Button } from "@/app/element/ui";
import type { NativeMemoryFile } from "./native-memory-file-view-state";

type Props = { t: NativeMemoryFile };

/** Edit while reading; Save, Cancel and whether there are unsaved changes while editing. */
function EditorActions(props: Props & { confirm: PaneConfirm }): JSX.Element {
    const { draft, history } = props.t;
    return (
        <div class="memory-editor-actions">
            <Show
                when={draft.editingAtom()}
                fallback={
                    <Button
                        density="compact"
                        class="memory-editor-btn"
                        disabled={history.contentAtom() === null}
                        onClick={props.t.startEdit}
                    >
                        Edit
                    </Button>
                }
            >
                <Button
                    density="compact"
                    tone="accent"
                    class="memory-editor-btn is-primary"
                    disabled={draft.savingAtom()}
                    onClick={() => void props.t.save()}
                    title={`Save (${keyLabel("mod+s")})`}
                >
                    {draft.savingAtom() ? "Saving…" : "Save"}
                </Button>
                <Button
                    density="compact"
                    class="memory-editor-btn"
                    disabled={draft.savingAtom()}
                    onClick={() => requestCancel({ isDirty: draft.dirtyAtom, onCancel: props.t.cancel, confirm: askDiscard(props.confirm) })}
                    title="Cancel (Esc)"
                >
                    Cancel
                </Button>
                <span class="memory-editor-status">{draft.dirtyAtom() ? "Unsaved changes" : "No changes"}</span>
            </Show>
        </div>
    );
}

/** The top region: the actions, a save's error, the conflict banner and the version history. */
export function FileTop(props: Props & { confirm: PaneConfirm }): JSX.Element {
    const { draft, history } = props.t;
    return (
        <>
            <EditorActions t={props.t} confirm={props.confirm} />
            <Show when={draft.errorAtom()}>
                <div class="memory-editor-error">{draft.errorAtom()}</div>
            </Show>
            <MemoryConflictBanner
                model={draft}
                toText={(v) => v}
                noun="this file"
                onDiscarded={() => void history.loadContent()}
            />
            <MemoryHistory model={history} revertDisabled={draft.savingAtom()} />
        </>
    );
}

/** The bottom region: the file's content, or the editor on the draft. */
export function FileBottom(props: Props): JSX.Element {
    const { draft, history, filename } = props.t;
    return (
        <MemoryContent
            content={history.contentAtom()}
            loading={history.contentLoadingAtom()}
            error={history.contentErrorAtom()}
            editing={draft.editingAtom()}
            autoFocus
            draft={draft.draftAtom() ?? ""}
            onDraftInput={(v) => draft.setDraft(v)}
            textareaLabel={`Edit ${filename}`}
        />
    );
}
