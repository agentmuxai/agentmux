// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import type { JSX } from "solid-js";
import { Show } from "solid-js";
import type { ForgeViewModel } from "../forge-model";

export function ContentEditor(props: {
    agentId: string;
    contentType: string;
    content: ForgeContent | undefined;
    model: ForgeViewModel;
}): JSX.Element {
    const saving = props.model.contentSavingAtom;
    const [editing, setEditing] = createSignal(false);
    const [draft, setDraft] = createSignal("");

    const currentContent = () => props.content?.content ?? "";
    const charCount = () => currentContent().length;

    const handleStartEdit = () => {
        setDraft(currentContent());
        setEditing(true);
    };

    const handleCancel = () => {
        setEditing(false);
        setDraft("");
    };

    const handleSave = async () => {
        await props.model.saveContent(props.agentId, props.contentType, draft());
        setEditing(false);
    };

    return (
        <Show when={editing()} fallback={
            <div class="forge-content-display">
                <Show when={currentContent()} fallback={
                    <div class="forge-content-empty">No content yet</div>
                }>
                    <pre class="forge-content-pre">{currentContent()}</pre>
                </Show>
                <div class="forge-content-display-footer">
                    <span class="forge-content-charcount">
                        {charCount() > 0 ? `${charCount()} chars` : ""}
                        {props.content?.updated_at ? ` \u2022 saved` : ""}
                    </span>
                    <button class="forge-btn-primary" onClick={handleStartEdit}>
                        Edit Content
                    </button>
                </div>
            </div>
        }>
            <div class="forge-content-editor">
                <textarea
                    class="forge-content-textarea"
                    value={draft()}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                    autofocus
                    spellcheck={false}
                />
                <div class="forge-content-editor-footer">
                    <span class="forge-content-charcount">{draft().length} chars</span>
                    <div class="forge-content-editor-actions">
                        <button class="forge-btn-primary" onClick={handleSave} disabled={saving()}>
                            {saving() ? "Saving..." : "Save"}
                        </button>
                        <button class="forge-btn-secondary" onClick={handleCancel} disabled={saving()}>
                            Cancel
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    );
}
