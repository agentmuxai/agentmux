// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the Editor draws around an open file: the Markdown mode toolbar and preview, the panel for a file that would not open, the language server's install banner and its status chip. */

import type { Accessor, JSX } from "solid-js";
import { Show } from "solid-js";
import { Markdown } from "@/app/element/markdown";
import { Button, IconButton, SegmentedControl } from "@/app/element/ui";
import { redactSecrets } from "@/app/errors/redact";
import { keyLabel } from "@/app/keybindings";
import type { EditorMode, EditorViewModel } from "./editor-model";
import { previewMedia } from "./editor-preview-media";
import { installHintFor } from "./lsp/install-hints";
import type { LspState } from "./lsp/lsp-client";

/** Preview, Source and Split, for a Markdown file. */
export function EditorModeToolbar(props: { model: EditorViewModel }): JSX.Element {
    const model = props.model;
    return (
            <div class="editor-mode-toolbar">
                <SegmentedControl<EditorMode>
                    density="compact"
                    ariaLabel="View mode"
                    options={[
                        { value: "preview", label: "Preview", title: `Rendered preview (${keyLabel("mod+shift+v")})` },
                        { value: "source", label: "Source", title: "Source editor" },
                        { value: "split", label: "Split", title: "Split view" },
                    ]}
                    value={model.editorMode()}
                    onChange={(mode) => model.setEditorMode(mode)}
                />
            </div>
    );
}

/** Full-pane panel for a file that failed to load: it replaces the CodeMirror body for the active tab while the error sticks. */
export function EditorErrorPanel(props: { model: EditorViewModel }): JSX.Element {
    const model = props.model;
    return (
            <div class="editor-error-panel" role="alert">
                <div class="editor-error-panel-icon" aria-hidden="true">⚠</div>
                <div class="editor-error-panel-title">Couldn't open file</div>
                <div class="editor-error-panel-path">{model.filePathAtom()}</div>
                <div class="editor-error-panel-message">{model.errorAtom()}</div>
                <Button
                    class="editor-error-panel-close"
                    onClick={() => {
                        const tab = model.activeTabAtom();
                        if (tab) model.closeTab(tab.id);
                    }}
                >
                    Close tab
                </Button>
            </div>
    );
}

/** The language server's binary isn't on PATH: how to install it. Dismissable per session. */
export function EditorLspBanner(props: {
    state: Accessor<LspState | null>;
    onDismiss: (language: string) => void;
    onCopy: (text: string) => void;
}): JSX.Element {
    return (
        <>
                    {(() => {
                        const state = props.state();
                        if (!state || state.kind !== "missing") return null;
                        const hint = installHintFor(state.language);
                        return (
                            <div class="editor-lsp-banner" role="status">
                                <div class="editor-lsp-banner-row">
                                    <span class="editor-lsp-banner-icon" aria-hidden="true">ⓘ</span>
                                    <span class="editor-lsp-banner-text">
                                        {hint?.serverName ?? state.language} not installed.
                                    </span>
                                    <IconButton
                                        icon="xmark"
                                        label="Dismiss banner for this session"
                                        density="compact"
                                        onClick={() => props.onDismiss(state.language)}
                                    />
                                </div>
                                <Show when={hint}>
                                    <div class="editor-lsp-banner-row editor-lsp-banner-install">
                                        <span>Install:</span>
                                        <code class="editor-lsp-banner-cmd">{hint!.install}</code>
                                        <Button
                                            density="compact"
                                            // SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md §5 — moved to the redacted path.
                                            onClick={() => props.onCopy(redactSecrets(hint!.install))}
                                            title="Copy install command"
                                        >
                                            Copy
                                        </Button>
                                        <a
                                            class="editor-lsp-banner-docs"
                                            href={hint!.docs}
                                            target="_blank"
                                            rel="noopener noreferrer"
                                        >
                                            Docs ↗
                                        </a>
                                    </div>
                                </Show>
                            </div>
                        );
                    })()}
        </>
    );
}

/** The preview beside (or in place of) the source of a Markdown file. */
export function EditorMarkdownPreview(props: { model: EditorViewModel; text: Accessor<string> }): JSX.Element {
    const model = props.model;
    return (
                    <div
                        class="editor-preview-pane"
                        style={
                            model.editorMode() === "split"
                                ? { height: `${model.previewHeightAtom()}px`, flex: "0 0 auto" }
                                : { flex: "1 1 auto" }
                        }
                    >
                        <div class="editor-preview-content">
                            <Markdown
                                textAtom={() => props.text()}
                                contentClass="editor-preview-markdown-content"
                                // Local images in the document render, relative to its
                                // folder; none for a document on an SSH host.
                                media={previewMedia(model.filePathAtom(), model.connection(), !!model.activeTabAtom()?.isScratch)}
                            />
                        </div>
                    </div>
    );
}

/** Bottom-of-pane indicator of the language server's state. */
export function EditorLspStatusChip(props: { kind: string; text: string }): JSX.Element {
    return (
            <div class="editor-lsp-status" data-kind={props.kind}>
                <span class="editor-lsp-status-dot" />
                <span class="editor-lsp-status-text">{props.text}</span>
            </div>
    );
}
