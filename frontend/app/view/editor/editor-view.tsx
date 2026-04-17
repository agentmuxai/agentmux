// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, onCleanup, onMount, Show, untrack, type JSX } from "solid-js";
import { EditorView, basicSetup } from "codemirror";
import { EditorState, type Extension } from "@codemirror/state";
import { keymap } from "@codemirror/view";
import { oneDark } from "@codemirror/theme-one-dark";
import { search } from "@codemirror/search";
import type { EditorViewModel } from "./editor-model";
import "./editor-view.scss";

// ── Language loader ─────────────────────────────────────────────────────────
// Lazy-load language extensions to keep initial bundle small.

async function loadLanguage(lang: string): Promise<Extension | null> {
    try {
        switch (lang) {
            case "typescript":
            case "javascript": {
                const { javascript } = await import("@codemirror/lang-javascript");
                return javascript({ typescript: lang === "typescript", jsx: true });
            }
            case "python": {
                const { python } = await import("@codemirror/lang-python");
                return python();
            }
            case "rust": {
                const { rust } = await import("@codemirror/lang-rust");
                return rust();
            }
            case "html": {
                const { html } = await import("@codemirror/lang-html");
                return html();
            }
            case "css": {
                const { css } = await import("@codemirror/lang-css");
                return css();
            }
            case "json": {
                const { json } = await import("@codemirror/lang-json");
                return json();
            }
            case "markdown": {
                const { markdown } = await import("@codemirror/lang-markdown");
                return markdown();
            }
            default:
                return null;
        }
    } catch {
        return null;
    }
}

// ── Editor View Component ───────────────────────────────────────────────────

export function EditorViewComponent(props: ViewComponentProps<EditorViewModel>): JSX.Element {
    const model = props.model;
    let containerRef: HTMLDivElement | undefined;
    let cmView: EditorView | null = null;
    const [fileInput, setFileInput] = createSignal("");

    // Build or rebuild CodeMirror when content/language changes
    const setupEditor = async (content: string, language: string, readOnly: boolean) => {
        if (!containerRef) return;

        // Destroy previous instance
        if (cmView) {
            cmView.destroy();
            cmView = null;
        }

        const extensions: Extension[] = [
            basicSetup,
            oneDark,
            search(),
            EditorView.lineWrapping,
            EditorView.updateListener.of((update) => {
                if (update.docChanged) {
                    model.onContentChange(update.state.doc.toString());
                }
            }),
            // Ctrl+S → save
            keymap.of([{
                key: "Mod-s",
                run: () => {
                    void model.saveFile();
                    return true;
                },
            }]),
        ];

        if (readOnly) {
            extensions.push(EditorState.readOnly.of(true));
        }

        // Load language extension
        const langExt = await loadLanguage(language);
        if (langExt) extensions.push(langExt);

        cmView = new EditorView({
            state: EditorState.create({
                doc: content,
                extensions,
            }),
            parent: containerRef,
        });
    };

    onMount(() => {
        const content = model.contentAtom();
        const lang = model.languageAtom();
        const readOnly = model.readOnlyAtom();
        if (content || model.filePathAtom()) {
            void setupEditor(content, lang, readOnly);
        }
    });

    // Rebuild only when a NEW file is opened (path changes), not on every
    // keystroke. contentAtom is updated by onContentChange on every keypress —
    // reading it inside a tracked effect would destroy+recreate CodeMirror
    // on every keystroke. untrack() prevents SolidJS from subscribing to
    // those inner reads.
    createEffect(() => {
        const _path = model.filePathAtom(); // reactive dependency
        const loading = model.loadingAtom(); // reactive dependency
        if (!loading && _path && containerRef) {
            untrack(() => {
                const content = model.contentAtom();
                const lang = model.languageAtom();
                const readOnly = model.readOnlyAtom();
                void setupEditor(content, lang, readOnly);
            });
        }
    });

    onCleanup(() => {
        cmView?.destroy();
        cmView = null;
    });

    const handleOpenFile = () => {
        const path = fileInput().trim();
        if (path) {
            void model.openFile(path);
        }
    };

    return (
        <div class="editor-view">
            <Show when={!model.filePathAtom()}>
                <div class="editor-open-prompt">
                    <div class="editor-open-label">Open a file</div>
                    <div class="editor-open-row">
                        <input
                            class="editor-open-input"
                            type="text"
                            placeholder="/path/to/file.ts"
                            value={fileInput()}
                            onInput={(e) => setFileInput(e.currentTarget.value)}
                            onKeyDown={(e) => {
                                if (e.key === "Enter") handleOpenFile();
                            }}
                        />
                        <button class="editor-open-btn" onClick={handleOpenFile}>
                            Open
                        </button>
                    </div>
                </div>
            </Show>

            <Show when={model.loadingAtom()}>
                <div class="editor-loading">Loading...</div>
            </Show>

            <Show when={model.errorAtom()}>
                <div class="editor-error">{model.errorAtom()}</div>
            </Show>

            <div
                class="editor-codemirror"
                ref={containerRef}
                style={{ display: model.filePathAtom() && !model.loadingAtom() ? "block" : "none" }}
            />
        </div>
    );
}
