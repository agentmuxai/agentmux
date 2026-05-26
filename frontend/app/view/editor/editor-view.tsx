// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Editor view — file-tree column on the left + CodeMirror on the right.
// Tree visibility toggled by the header chevron (model.treeExpandedAtom).
// Spec: specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md

import { createEffect, createSignal, onCleanup, onMount, Show, untrack, type JSX } from "solid-js";
import { EditorView, basicSetup } from "codemirror";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { keymap } from "@codemirror/view";
import { oneDark } from "@codemirror/theme-one-dark";
import { search } from "@codemirror/search";
import { lintGutter } from "@codemirror/lint";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { settingsAtom } from "@/store/global";
import type { EditorViewModel } from "./editor-model";
import { FileTree } from "./file-tree";
import { LspClient, type LspState } from "./lsp/lsp-client";
import { lspDiagnosticsExtension } from "./lsp/lsp-extensions";
import { installHintFor, isLspSupportedLanguage } from "./lsp/install-hints";
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

    // ── LSP integration (Phase 1 — diagnostics for TS/JS) ────────────
    // One LspClient per (pane, file) — replaced when the file changes
    // or the language switches. Diagnostics flow through a Compartment
    // so they can be reconfigured without rebuilding CodeMirror.
    let lspClient: LspClient | null = null;
    let lspDiagUnsub: (() => void) | null = null;
    let lspStateUnsub: (() => void) | null = null;
    let lspChangeDebounce: ReturnType<typeof setTimeout> | null = null;
    const [lspState, setLspState] = createSignal<LspState | null>(null);
    const lintCompartment = new Compartment();

    const teardownLsp = async (): Promise<void> => {
        if (lspChangeDebounce) {
            clearTimeout(lspChangeDebounce);
            lspChangeDebounce = null;
        }
        if (lspDiagUnsub) {
            lspDiagUnsub();
            lspDiagUnsub = null;
        }
        if (lspStateUnsub) {
            lspStateUnsub();
            lspStateUnsub = null;
        }
        const old = lspClient;
        lspClient = null;
        setLspState(null);
        if (old) {
            await old.dispose();
        }
    };

    const startLspIfSupported = async (
        filePath: string,
        language: string,
        content: string,
    ): Promise<void> => {
        // Bail early on unsupported language or master kill switch — but
        // tear down any client carried over from the previous file first.
        // Otherwise a non-LSP file leaves the prior client (and its
        // server-side refcount) alive, debounced didChange calls keep
        // firing, and the status chip/banner still reflect the old file.
        if (!isLspSupportedLanguage(language) || settingsAtom()?.["editor:lsp.enabled"] === false) {
            await teardownLsp();
            return;
        }

        // Reuse the existing client when only the file changed (same language
        // AND the new file lives under the server's workspace root). The
        // refcount-sharing design relies on this — tearing down per-file
        // would defeat it and respawn the server every navigation. The
        // diagnostics extension is re-wired because setupEditor rebuilt cmView.
        const existing = lspClient;
        if (
            existing &&
            existing.language === language &&
            existing.getState().kind === "ready" &&
            workspaceCovers(existing.getWorkspaceRoot(), filePath)
        ) {
            const prevUri = existing.getOpenedFileUri();
            if (prevUri) {
                // Best-effort close of the previous file.
                void existing.didClose(decodeFileUri(prevUri));
            }
            await existing.didOpen(filePath, content, language);
            if (cmView) {
                if (lspDiagUnsub) lspDiagUnsub();
                const [ext, unsub] = lspDiagnosticsExtension(cmView, existing);
                lspDiagUnsub = unsub;
                cmView.dispatch({ effects: lintCompartment.reconfigure(ext) });
            }
            return;
        }

        await teardownLsp();

        const client = new LspClient(language, filePath);
        lspClient = client;
        lspStateUnsub = client.onStateChange(setLspState);

        const ok = await client.start();
        if (!ok) {
            // Status is "missing" or "crashed" — banner takes over from here.
            return;
        }
        await client.didOpen(filePath, content, language);
        if (cmView && lspClient === client) {
            const [ext, unsub] = lspDiagnosticsExtension(cmView, client);
            lspDiagUnsub = unsub;
            cmView.dispatch({ effects: lintCompartment.reconfigure(ext) });
        }
    };

    // True when `filePath` lives under `root` — handles both POSIX and
    // Windows separators, case-insensitive on Windows drives.
    const workspaceCovers = (root: string | null, filePath: string): boolean => {
        if (!root) return false;
        const normRoot = root.replace(/\\/g, "/").replace(/\/+$/, "");
        const normFile = filePath.replace(/\\/g, "/");
        const isWin = /^[a-zA-Z]:\//.test(normRoot);
        const r = isWin ? normRoot.toLowerCase() : normRoot;
        const f = isWin ? normFile.toLowerCase() : normFile;
        return f === r || f.startsWith(r + "/");
    };

    // file:// → OS path (inverse of pathToFileUri in lsp-client.ts).
    const decodeFileUri = (uri: string): string => {
        if (uri.startsWith("file:///") && /^file:\/\/\/[a-zA-Z]:/.test(uri)) {
            // Windows: strip "file:///", restore backslashes
            return uri.slice(8).replace(/\//g, "\\");
        }
        if (uri.startsWith("file://")) return uri.slice(7);
        return uri;
    };

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
            lintGutter(),
            lintCompartment.of([]),
            EditorView.lineWrapping,
            EditorView.updateListener.of((update) => {
                if (update.docChanged) {
                    const content = update.state.doc.toString();
                    model.onContentChange(content);
                    // Debounced LSP didChange — Phase 1 ships full-sync,
                    // which is the simplest and works on every server.
                    if (lspChangeDebounce) clearTimeout(lspChangeDebounce);
                    lspChangeDebounce = setTimeout(() => {
                        if (lspClient && lspClient.getState().kind === "ready") {
                            void lspClient.didChange(model.filePathAtom(), content);
                        }
                    }, 250);
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
        // Load file-tree roots from backend: $HOME (auto-expanded) +
        // every reachable drive/mount (sibling roots, collapsed).
        void RpcApi.GetEditorRootsCommand(TabRpcClient, {}).then((res) => {
            if (res?.home) {
                void model.treeModel.setRootsAndLoad(res.home, res.drives ?? []);
            }
        });

        const content = model.contentAtom();
        const lang = model.languageAtom();
        const readOnly = model.readOnlyAtom();
        if (content || model.filePathAtom()) {
            void setupEditor(content, lang, readOnly);
        }
    });

    // Resize-handle drag — tracks global mousemove until release so the user
    // can drag past the handle without losing the grip. Live updates the
    // tree-width signal; persists once on mouseup.
    const handleResizeMouseDown = (e: MouseEvent) => {
        e.preventDefault();
        const startX = e.clientX;
        const startWidth = model.treeWidthAtom();
        const onMove = (ev: MouseEvent) => {
            model.setTreeWidth(startWidth + (ev.clientX - startX));
        };
        const onUp = () => {
            document.removeEventListener("mousemove", onMove);
            document.removeEventListener("mouseup", onUp);
            document.body.style.cursor = "";
            document.body.style.userSelect = "";
            void model.commitTreeWidth();
        };
        document.body.style.cursor = "col-resize";
        document.body.style.userSelect = "none";
        document.addEventListener("mousemove", onMove);
        document.addEventListener("mouseup", onUp);
    };

    // Rebuild only when a NEW file is opened (path changes), not on every
    // keystroke. contentAtom is updated by onContentChange on every keypress —
    // reading it inside a tracked effect would destroy+recreate CodeMirror
    // on every keystroke. untrack() prevents SolidJS from subscribing to
    // those inner reads.
    createEffect(() => {
        const path = model.filePathAtom(); // reactive dependency
        const loading = model.loadingAtom(); // reactive dependency
        if (!loading && path && containerRef) {
            untrack(() => {
                const content = model.contentAtom();
                const lang = model.languageAtom();
                const readOnly = model.readOnlyAtom();
                void setupEditor(content, lang, readOnly).then(() => {
                    // Kick off LSP after CodeMirror is up so the Compartment
                    // reconfigure has somewhere to land.
                    void startLspIfSupported(path, lang, content);
                });
            });
        }
    });

    onCleanup(() => {
        void teardownLsp();
        cmView?.destroy();
        cmView = null;
    });

    const handleOpenFile = () => {
        const path = fileInput().trim();
        if (path) {
            void model.openFile(path);
            setFileInput("");
        }
    };

    // ── LSP install banner state ──────────────────────────────────────
    // Dismissed-for-session list keyed by language. Allows the operator
    // to silence the banner per session; it returns on next launch if the
    // binary is still missing.
    const [dismissedLanguages, setDismissedLanguages] = createSignal<Set<string>>(new Set());
    const lspBannerVisible = (): boolean => {
        const s = lspState();
        if (!s || s.kind !== "missing") return false;
        return !dismissedLanguages().has(s.language);
    };
    const dismissBanner = (language: string) => {
        const next = new Set(dismissedLanguages());
        next.add(language);
        setDismissedLanguages(next);
    };
    const copyToClipboard = (text: string) => {
        try {
            void navigator.clipboard?.writeText(text);
        } catch {
            // Clipboard might not be available — silently ignore
        }
    };
    const statusChipText = (): string => {
        const s = lspState();
        if (!s) return "";
        const lang = model.languageAtom();
        switch (s.kind) {
            case "starting":
                return `${lang}: starting…`;
            case "initializing":
                return `${lang}: initializing…`;
            case "ready":
                return `${lang}: ready`;
            case "missing":
                return `${lang}: not installed`;
            case "crashed":
                return `${lang}: error`;
            case "disposed":
                return "";
        }
    };
    const statusChipKind = (): string => {
        const s = lspState();
        return s?.kind ?? "none";
    };

    return (
        <div
            class="editor-view"
            classList={{ "editor-view--tree-collapsed": !model.treeExpandedAtom() }}
        >
            <Show when={model.treeExpandedAtom()}>
                <div
                    class="editor-tree-column"
                    style={{
                        width: `${model.treeWidthAtom()}px`,
                        flex: `0 0 ${model.treeWidthAtom()}px`,
                    }}
                >
                    <FileTree
                        model={model.treeModel}
                        activeFilePath={model.filePathAtom()}
                        showHidden={model.showHiddenAtom()}
                        onFileClick={(path) => void model.openFile(path)}
                        onToggleHidden={() => void model.toggleShowHidden()}
                    />
                    <Show when={!model.filePathAtom()}>
                        <div class="editor-tree-path-input">
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
                            <button
                                class="editor-open-btn"
                                onClick={handleOpenFile}
                                disabled={!fileInput().trim()}
                            >
                                Open
                            </button>
                        </div>
                    </Show>
                </div>
                <div
                    class="editor-tree-resize-handle"
                    onMouseDown={handleResizeMouseDown}
                    title="Drag to resize file tree"
                />
            </Show>

            <div class="editor-main-column">
                <Show when={model.loadingAtom()}>
                    <div class="editor-loading">Loading...</div>
                </Show>

                <Show when={model.errorAtom()}>
                    <div class="editor-error">{model.errorAtom()}</div>
                </Show>

                {/* LSP install banner — appears when the language server's binary
                    isn't on PATH. Dismissable per session via the X button. */}
                <Show when={lspBannerVisible()}>
                    {(() => {
                        const state = lspState();
                        if (!state || state.kind !== "missing") return null;
                        const hint = installHintFor(state.language);
                        return (
                            <div class="editor-lsp-banner" role="status">
                                <div class="editor-lsp-banner-row">
                                    <span class="editor-lsp-banner-icon" aria-hidden="true">ⓘ</span>
                                    <span class="editor-lsp-banner-text">
                                        {hint?.serverName ?? state.language} not installed.
                                    </span>
                                    <button
                                        class="editor-lsp-banner-dismiss"
                                        aria-label="Dismiss banner for this session"
                                        title="Dismiss for this session"
                                        onClick={() => dismissBanner(state.language)}
                                    >
                                        ×
                                    </button>
                                </div>
                                <Show when={hint}>
                                    <div class="editor-lsp-banner-row editor-lsp-banner-install">
                                        <span>Install:</span>
                                        <code class="editor-lsp-banner-cmd">{hint!.install}</code>
                                        <button
                                            class="editor-lsp-banner-copy"
                                            onClick={() => copyToClipboard(hint!.install)}
                                            title="Copy install command"
                                        >
                                            Copy
                                        </button>
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
                </Show>

                <Show
                    when={model.filePathAtom() && !model.loadingAtom()}
                    fallback={
                        <Show when={!model.treeExpandedAtom()}>
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
                                <div class="editor-open-hint">
                                    Show the file tree from the header chevron to browse your files.
                                </div>
                            </div>
                        </Show>
                    }
                >
                    <div class="editor-codemirror" ref={containerRef} />
                </Show>

                {/* LSP status chip — bottom-of-pane indicator. Only shown when
                    there's an active client (i.e. an LSP-supported file is open). */}
                <Show when={lspState() && lspState()?.kind !== "disposed"}>
                    <div class="editor-lsp-status" data-kind={statusChipKind()}>
                        <span class="editor-lsp-status-dot" />
                        <span class="editor-lsp-status-text">{statusChipText()}</span>
                    </div>
                </Show>
            </div>
        </div>
    );
}
