// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Editor view — file-tree column on the left + CodeMirror on the right.
// Tree visibility toggled by the header chevron (model.treeExpandedAtom).
// Spec: specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md

import { createEffect, createSignal, onCleanup, onMount, Show, untrack, type JSX } from "solid-js";
import { ContextMenu, type ContextMenuItem } from "@/app/components/context-menu";
import { ConfirmDialog } from "@/app/components/confirm-dialog";
import { EditorView, basicSetup } from "codemirror";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { keymap } from "@codemirror/view";
import { search, openSearchPanel } from "@codemirror/search";
import { lintGutter } from "@codemirror/lint";
import { editorTheme } from "./editor-theme";
import { Markdown } from "@/app/element/markdown";
import brainLogoSvg from "@/app/asset/logo-brain.svg?raw";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { settingsAtom } from "@/store/global";
import type { EditorViewModel } from "./editor-model";
import { EditorTabStrip } from "./editor-tab-strip";
import { FileTree } from "./file-tree";
import { LspClient, type LspState } from "./lsp/lsp-client";
import { lspDiagnosticsExtension } from "./lsp/lsp-extensions";
import { installHintFor, isLspSupportedLanguage } from "./lsp/install-hints";
import { writeText as clipboardWriteText } from "@/util/clipboard";
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
    // Reactive ref: the markdown preview seeds `liveDoc` from the tab-change
    // effect, which guards on the container being mounted. On first open the
    // container mounts only AFTER content finishes loading (the body <Show>
    // gates on !loadingAtom), so a plain `let` ref left the effect early-
    // returning before the container existed → blank preview until a manual
    // Source/Preview toggle. A signal makes the effect re-run when the
    // container mounts, seeding the preview reliably.
    const [containerRef, setContainerRef] = createSignal<HTMLDivElement>();
    let rootRef: HTMLDivElement | undefined;
    let cmView: EditorView | null = null;

    // Platform-aware modifier glyph for the empty-state shortcut hints.
    const isMac = /mac/i.test(navigator.platform || navigator.userAgent || "");
    const MOD = isMac ? "⌘" : "Ctrl";
    // The handful of shortcuts worth surfacing on the empty editor.
    const emptyShortcuts: { keys: string[]; label: string }[] = [
        { keys: [MOD, "S"], label: "Save" },
        { keys: [MOD, "F"], label: "Find" },
        { keys: [MOD, "⇧", "V"], label: "Toggle .md preview" },
        { keys: [MOD, "+ / −"], label: "Zoom" },
    ];

    // ── Markdown rendered/source view ────────────────────────────────────────
    // Live CodeMirror doc text for the active tab — the markdown preview binds
    // to this, NOT model.contentAtom(): that memo only recomputes on file
    // load / tab change (onContentChange deliberately doesn't bump it on
    // keystrokes), so it would render stale pre-edit text. We seed liveDoc
    // whenever CM is (re)built or restored, and update it on every doc change.
    const [liveDoc, setLiveDoc] = createSignal("");
    const isMarkdown = (): boolean => model.languageAtom() === "markdown";

    // Ctrl+Wheel zoom — plugs into the universal zoom system (term:zoom on
    // block meta, same path used by terminal/agent/swarm). Capture phase so
    // we intercept before CodeMirror's bubble-phase wheel; preventDefault
    // suppresses CEF's native Ctrl+Scroll page zoom. The resulting zoom
    // factor is applied as a CSS `zoom` property on .editor-view (see below),
    // scaling the entire subtree uniformly.
    onMount(() => {
        if (!rootRef) return;
        const handleCtrlWheel = (ev: WheelEvent) => {
            if (!ev.ctrlKey) return;
            ev.preventDefault();
            ev.stopPropagation();
            const STEP = 0.1;
            const current = model.zoomAtom();
            const next = Math.max(0.5, Math.min(2.0, Math.round((current + (ev.deltaY > 0 ? -STEP : STEP)) * 100) / 100));
            void RpcApi.SetMetaCommand(TabRpcClient, {
                oref: `block:${model.blockId}`,
                meta: { "term:zoom": next === 1.0 ? null : next },
            });
        };
        rootRef.addEventListener("wheel", handleCtrlWheel, { passive: false, capture: true });
        onCleanup(() => rootRef?.removeEventListener("wheel", handleCtrlWheel, { capture: true }));

        // Editor-pane key handling, capture phase. We intercept here so the
        // app-wide Cmd/Ctrl+F binding (keymodel.ts → universal block-search,
        // which the editor doesn't implement) never swallows these — and so a
        // single deterministic path handles them regardless of CM focus state.
        const handleEditorKeys = (ev: KeyboardEvent) => {
            const mod = ev.ctrlKey || ev.metaKey;
            if (!mod) return;
            // Ctrl/Cmd+F → CodeMirror's native find panel (find/replace, regex,
            // case, whole-word). Only in source view — in rendered markdown
            // there's nothing editable to search. See
            // docs/specs/SPEC_EDITOR_AND_APP_FIND_2026_06_17.md.
            if (!ev.shiftKey && (ev.key === "f" || ev.key === "F")) {
                if (!cmView || model.editorMode() === "preview") return;
                ev.preventDefault();
                ev.stopPropagation();
                openSearchPanel(cmView);
                return;
            }
            // Mod-Shift-V: toggle between preview and source modes.
            if (ev.shiftKey && (ev.key === "v" || ev.key === "V")) {
                if (!isMarkdown()) return;
                ev.preventDefault();
                ev.stopPropagation();
                model.toggleEditorMode();
            }
        };
        rootRef.addEventListener("keydown", handleEditorKeys, { capture: true });
        onCleanup(() => rootRef?.removeEventListener("keydown", handleEditorKeys, { capture: true }));
    });

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
    // Word wrap toggle (right-click menu → editor-model.ts's
    // getBodyContextMenuItems). Reconfigured live on toggle (see the
    // createEffect below) and resynced on every tab switch — it's a
    // pane-wide setting, not per-tab, so a stale per-tab CodeMirror-state
    // snapshot (cmStates) must not be allowed to reintroduce an old value.
    const wordWrapCompartment = new Compartment();

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
            // Cancel any pending didChange from the OUTGOING file. The
            // debounced callback reads model.filePathAtom() at fire time, so
            // a stale tick would send the previous file's text under the new
            // file's URI and desync the server until the next keystroke.
            if (lspChangeDebounce) {
                clearTimeout(lspChangeDebounce);
                lspChangeDebounce = null;
            }
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
    // Percent-decodes each segment to undo the encoding applied on the way out.
    const decodeFileUri = (uri: string): string => {
        const isWin = uri.startsWith("file:///") && /^file:\/\/\/[a-zA-Z]:/.test(uri);
        const body = uri.startsWith("file:///")
            ? uri.slice(isWin ? 8 : 7) // Windows strips the leading "/" before the drive
            : uri.startsWith("file://")
              ? uri.slice(7)
              : uri;
        const decoded = body
            .split("/")
            .map((seg) => {
                try {
                    return decodeURIComponent(seg);
                } catch {
                    return seg;
                }
            })
            .join("/");
        return isWin ? decoded.replace(/\//g, "\\") : decoded;
    };

    // Per-tab CodeMirror state cache — preserves cursor, selection, scroll,
    // undo history across tab switches. Populated on tab switch (snapshot of
    // outgoing), cleared on TabClosed via the slice event subscription
    // wired below.
    const cmStates = new Map<string, EditorState>();
    let activeTabIdForCm: string | null = null;

    // Build or rebuild CodeMirror when the active tab changes
    const setupEditor = async (content: string, language: string, readOnly: boolean) => {
        const container = containerRef();
        if (!container) return;

        // Destroy previous instance
        if (cmView) {
            cmView.destroy();
            cmView = null;
        }

        const extensions: Extension[] = [
            basicSetup,
            editorTheme,
            search(),
            // lintGutter installs the lint state field that backs the inline
            // diagnostic underlines (LSP pushes via setDiagnostics). We keep
            // it for that field but hide its gutter column in CSS — there's
            // no debugger/breakpoint margin, so the numbers sit tight to code.
            lintGutter(),
            lintCompartment.of([]),
            wordWrapCompartment.of(model.wordWrapAtom() ? [EditorView.lineWrapping] : []),
            EditorView.updateListener.of((update) => {
                if (update.docChanged) {
                    const content = update.state.doc.toString();
                    model.onContentChange(content);
                    setLiveDoc(content); // keep the markdown preview in sync
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
            // Ctrl+S → save; on scratch tabs triggers Save As instead.
            // Ctrl+Shift+S → Save As for scratch tabs only (Phase 1; non-scratch Save As is Phase 2).
            keymap.of([
                {
                    key: "Mod-s",
                    run: () => {
                        if (model.activeTabAtom()?.isScratch) {
                            triggerSaveAs();
                        } else {
                            void model.saveFile();
                        }
                        return true;
                    },
                },
                {
                    key: "Mod-Shift-s",
                    run: () => {
                        // Phase 1: Save As is only implemented for scratch tabs.
                        // Don't swallow the key for non-scratch tabs so the OS
                        // default (or a future handler) can still see it.
                        if (!model.activeTabAtom()?.isScratch) return false;
                        triggerSaveAs();
                        return true;
                    },
                },
            ]),
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
            parent: container,
        });
        setLiveDoc(content); // seed preview from the freshly-built doc
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
            setLiveDoc(content); // seed before async setupEditor to avoid a blank preview
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
            // document mousemove coords are in viewport CSS pixels; divide by
            // zoom so the delta maps correctly to local CSS pixels inside the
            // zoomed .editor-view element.
            model.setTreeWidth(startWidth + (ev.clientX - startX) / model.zoomAtom());
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

    // Preview-panel resize — drag up = taller, drag down = shorter.
    const handlePreviewResizeMouseDown = (e: MouseEvent) => {
        e.preventDefault();
        const startY = e.clientY;
        const startH = model.previewHeightAtom();
        const onMove = (ev: MouseEvent) => {
            model.setPreviewHeight(startH + (startY - ev.clientY) / model.zoomAtom());
        };
        const onUp = () => {
            document.removeEventListener("mousemove", onMove);
            document.removeEventListener("mouseup", onUp);
            document.body.style.cursor = "";
            document.body.style.userSelect = "";
            void model.commitPreviewHeight();
        };
        document.body.style.cursor = "row-resize";
        document.body.style.userSelect = "none";
        document.addEventListener("mousemove", onMove);
        document.addEventListener("mouseup", onUp);
    };

    // Rebuild CodeMirror on tab change. Tracks activeIdAtom (not just
    // filePathAtom) so we get a unique trigger per tab — multiple tabs
    // could share the same path in pathological cases, but each has its own
    // id. The outgoing tab's state is snapshotted into cmStates BEFORE the
    // rebuild so we can restore it on the next switch back.
    createEffect(() => {
        const activeId = model.activeIdAtom(); // reactive: tab change
        const loading = model.loadingAtom(); // reactive: wait for content
        // containerRef() is reactive: when the body <Show> mounts the container
        // after content loads, this effect re-runs and proceeds (fixes the
        // first-open blank-preview race).
        if (!activeId || loading || !containerRef()) return;

        untrack(() => {
            const path = model.filePathAtom();
            const content = model.contentAtom();
            const lang = model.languageAtom();
            const readOnly = model.readOnlyAtom();

            // Snapshot outgoing tab's CodeMirror state for cursor/scroll/
            // undo preservation. Skipped on first mount (no prevId).
            if (activeTabIdForCm && activeTabIdForCm !== activeId && cmView) {
                cmStates.set(activeTabIdForCm, cmView.state);
            }
            activeTabIdForCm = activeId;

            // If we have a saved state for this tab, restore via setState
            // on the existing cmView (no destroy). Otherwise build fresh.
            const saved = cmStates.get(activeId);
            if (saved && cmView) {
                cmView.setState(saved);
                // Resync the pane-wide word-wrap setting — the restored state
                // was snapshotted with whatever wrap value was active *at
                // that time*, which may be stale if the user toggled it via
                // another tab since.
                cmView.dispatch({
                    effects: wordWrapCompartment.reconfigure(model.wordWrapAtom() ? [EditorView.lineWrapping] : []),
                });
                setLiveDoc(saved.doc.toString()); // seed preview from restored doc
                // Re-wire LSP for the now-active file's content.
                void startLspIfSupported(path, lang, content);
                return;
            }
            setLiveDoc(content); // seed before async setupEditor to avoid a blank preview
            void setupEditor(content, lang, readOnly).then(() => {
                void startLspIfSupported(path, lang, content);
            });
        });
    });

    // Live-toggle word wrap (right-click menu → model.toggleWordWrap()) on
    // whichever tab is currently showing, without rebuilding CodeMirror.
    createEffect(() => {
        const wrap = model.wordWrapAtom();
        if (!cmView) return;
        cmView.dispatch({ effects: wordWrapCompartment.reconfigure(wrap ? [EditorView.lineWrapping] : []) });
    });

    // Clear cached CodeMirror state when its tab closes, so re-opening
    // the same file later starts fresh (matches user expectation —
    // closed-then-reopened ≠ "still has unsaved changes").
    const unsubSliceEvents = model.onSliceEvent((event) => {
        if (event.type === "TabClosed") {
            cmStates.delete(event.tabId);
        }
    });

    onCleanup(() => {
        void teardownLsp();
        unsubSliceEvents();
        cmStates.clear();
        cmView?.destroy();
        cmView = null;
    });

    // Empty-editor state — faded brain mark + the best few shortcuts. Shown
    // in the main column whenever no file is open (replaces the old path
    // input; the file tree is the way to open files).
    const EmptyEditor = () => (
        <div class="editor-empty">
            {/* eslint-disable-next-line solid/no-innerhtml */}
            <div class="editor-empty-logo" innerHTML={brainLogoSvg} aria-hidden="true" />
            <div class="editor-empty-shortcuts">
                {emptyShortcuts.map((s) => (
                    <div class="editor-empty-shortcut">
                        <span class="editor-empty-keys">
                            {s.keys.map((k) => (
                                <kbd>{k}</kbd>
                            ))}
                        </span>
                        <span class="editor-empty-label">{s.label}</span>
                    </div>
                ))}
            </div>
            <Show when={!model.treeExpandedAtom()}>
                <div class="editor-empty-hint">Open the file tree (chevron) to browse files.</div>
            </Show>
        </div>
    );

    // ── Save As flow (scratch tabs) ───────────────────────────────────
    const [saveAsTabId, setSaveAsTabId] = createSignal<string | null>(null);

    const triggerSaveAs = () => {
        const tab = model.activeTabAtom();
        if (tab?.isScratch) setSaveAsTabId(tab.id);
    };

    const handleSaveAsConfirm = async (path: string) => {
        setSaveAsTabId(null);
        if (path) await model.saveFileAs(path);
    };

    // ── File-tree context menu ────────────────────────────────────────
    const [ctxMenu, setCtxMenu] = createSignal<{ items: ContextMenuItem[]; x: number; y: number } | null>(null);
    const [renamingPath, setRenamingPath] = createSignal<string | null>(null);
    const [newEntry, setNewEntry] = createSignal<{ parentPath: string; kind: "file" | "dir" } | null>(null);
    const [deleteConfirm, setDeleteConfirm] = createSignal<{
        title: string;
        message: string;
        onConfirm: () => void;
    } | null>(null);

    const buildContextMenuItems = (path: string | null, isDir: boolean): ContextMenuItem[] => {
        if (!path) {
            // Background right-click → tree-level actions.
            const homeRoot = model.treeModel.rootsAtom().find((r) => r.isHome)?.path;
            if (!homeRoot) {
                return [
                    { type: "action", label: "Refresh", onSelect: () => void model.treeModel.refresh() },
                ];
            }
            return [
                { type: "action", label: "New File…", onSelect: () => { void model.treeModel.expandFolder(homeRoot); setNewEntry({ parentPath: homeRoot, kind: "file" }); } },
                { type: "action", label: "New Folder…", onSelect: () => { void model.treeModel.expandFolder(homeRoot); setNewEntry({ parentPath: homeRoot, kind: "dir" }); } },
                { type: "separator" },
                { type: "action", label: "Refresh", onSelect: () => void model.treeModel.refresh() },
            ];
        }
        const name = path.split(/[/\\]/).pop() ?? path;
        if (isDir) {
            const isRoot = model.treeModel.rootsAtom().some((r) => r.path === path);
            const items: ContextMenuItem[] = [
                { type: "action", label: "New File…", onSelect: () => { void model.treeModel.expandFolder(path); setNewEntry({ parentPath: path, kind: "file" }); } },
                { type: "action", label: "New Folder…", onSelect: () => { void model.treeModel.expandFolder(path); setNewEntry({ parentPath: path, kind: "dir" }); } },
                { type: "separator" },
                { type: "action", label: "Open in Terminal", onSelect: () => void model.openInTerminal(path) },
                { type: "action", label: "Reveal in Explorer", onSelect: () => void model.revealInExplorer(path) },
                { type: "action", label: "Collapse Folder", onSelect: () => model.treeModel.collapseFolder(path) },
            ];
            if (!isRoot) {
                items.push({ type: "separator" });
                items.push({ type: "action", label: "Rename…", shortcut: "F2", onSelect: () => setRenamingPath(path) });
                items.push({
                    type: "action",
                    label: "Delete",
                    danger: true,
                    onSelect: () => void model.deleteFile(path, true, (proceed) => {
                        setDeleteConfirm({
                            title: `Delete folder "${name}"?`,
                            message: `This will permanently delete "${name}" and all its contents. This cannot be undone.`,
                            onConfirm: proceed,
                        });
                    }),
                });
            }
            return items;
        }
        return [
            { type: "action", label: "Open", onSelect: () => void model.openFile(path) },
            { type: "action", label: "Open to the Side", onSelect: () => void model.openToTheSide(path) },
            { type: "action", label: "Open in New Tab", onSelect: () => void model.openInNewTab(path) },
            { type: "separator" },
            { type: "action", label: "Copy Path", onSelect: () => void copyToClipboard(path) },
            { type: "action", label: "Copy Relative Path", onSelect: () => {
                const root = model.treeModel.rootsAtom().find((r) => path === r.path || path.startsWith(r.path + "/") || path.startsWith(r.path + "\\"));
                const rel = root ? path.slice(root.path.length).replace(/^[/\\]/, "") : path;
                void copyToClipboard(rel);
            }},
            { type: "separator" },
            { type: "action", label: "Reveal in Explorer", onSelect: () => void model.revealInExplorer(path) },
            { type: "separator" },
            { type: "action", label: "Rename…", shortcut: "F2", onSelect: () => setRenamingPath(path) },
            {
                type: "action",
                label: "Delete",
                danger: true,
                onSelect: () => void model.deleteFile(path, false, (proceed) => {
                    setDeleteConfirm({
                        title: `Delete "${name}"?`,
                        message: `This will permanently delete "${name}". This cannot be undone.`,
                        onConfirm: proceed,
                    });
                }),
            },
        ];
    };

    const handleTreeContextMenu = (path: string | null, isDir: boolean, e: MouseEvent) => {
        setCtxMenu({ items: buildContextMenuItems(path, isDir), x: e.clientX, y: e.clientY });
    };

    const handleRenameConfirm = async (path: string, newName: string) => {
        setRenamingPath(null);
        await model.renameFile(path, newName);
    };

    const handleNewEntryConfirm = async (parentPath: string, name: string, kind: "file" | "dir") => {
        setNewEntry(null);
        try {
            if (kind === "file") await model.createFile(parentPath, name);
            else await model.createDir(parentPath, name);
        } catch {
            // Error already logged in model; tree will re-sync on next refresh.
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
        // Route through the CEF clipboard wrapper — navigator.clipboard is
        // blocked under CEF's Permissions-Policy. See SPEC_UNIFIED_CLIPBOARD_2026_05_18.md §3.3.
        void clipboardWriteText(text).catch(() => {
            // Clipboard might not be available — silently ignore
        });
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
            ref={(el) => { rootRef = el; }}
            class="editor-view"
            classList={{ "editor-view--tree-collapsed": !model.treeExpandedAtom() }}
            style={{ zoom: model.zoomAtom() }}
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
                        onFileClick={(path) => void model.openFilePreview(path)}
                        onFileDblClick={(path) => void model.openFile(path)}
                        onToggleHidden={() => void model.toggleShowHidden()}
                        onContextMenu={handleTreeContextMenu}
                        renamingPath={renamingPath()}
                        onRenameConfirm={(path, name) => void handleRenameConfirm(path, name)}
                        onRenameCancel={() => setRenamingPath(null)}
                        newEntry={newEntry()}
                        onNewEntryConfirm={(parent, name, kind) => void handleNewEntryConfirm(parent, name, kind)}
                        onNewEntryCancel={() => setNewEntry(null)}
                        onStartRename={(path) => setRenamingPath(path)}
                    />
                </div>
                <div
                    class="editor-tree-resize-handle"
                    onMouseDown={handleResizeMouseDown}
                    title="Drag to resize file tree"
                />
            </Show>

            <div class="editor-main-column">
                <Show when={model.tabsAtom().length > 0}>
                    <div class="editor-tab-strip-row">
                        <EditorTabStrip
                            model={model}
                            saveAsTabId={saveAsTabId()}
                            onSaveAsConfirm={(path) => void handleSaveAsConfirm(path)}
                            onSaveAsCancel={() => setSaveAsTabId(null)}
                        />
                    </div>
                </Show>

                <Show when={isMarkdown() && model.tabsAtom().length > 0}>
                    <div class="editor-mode-toolbar" role="group" aria-label="View mode">
                        <button
                            type="button"
                            class="editor-mode-btn"
                            classList={{ active: model.editorMode() === "preview" }}
                            onClick={() => model.setEditorMode("preview")}
                            title="Rendered preview (Mod+Shift+V)"
                        >Preview</button>
                        <button
                            type="button"
                            class="editor-mode-btn"
                            classList={{ active: model.editorMode() === "source" }}
                            onClick={() => model.setEditorMode("source")}
                            title="Source editor"
                        >Source</button>
                        <button
                            type="button"
                            class="editor-mode-btn"
                            classList={{ active: model.editorMode() === "split" }}
                            onClick={() => model.setEditorMode("split")}
                            title="Split view"
                        >Split</button>
                    </div>
                </Show>

                <Show when={model.loadingAtom()}>
                    <div class="editor-loading">Loading...</div>
                </Show>

                {/* Full-pane error panel when a file failed to load (e.g.
                    invalid path, permission denied, binary). Replaces the
                    CodeMirror body for the active tab while the error sticks.
                    Operational errors that occur with content already loaded
                    (e.g. save failures) still surface via the top banner
                    below. */}
                <Show when={model.errorAtom() && model.activeTabAtom() && !model.activeTabAtom()?.contentLoaded}>
                    <div class="editor-error-panel" role="alert">
                        <div class="editor-error-panel-icon" aria-hidden="true">⚠</div>
                        <div class="editor-error-panel-title">Couldn't open file</div>
                        <div class="editor-error-panel-path">{model.filePathAtom()}</div>
                        <div class="editor-error-panel-message">{model.errorAtom()}</div>
                        <button
                            class="editor-error-panel-close"
                            onClick={() => {
                                const tab = model.activeTabAtom();
                                if (tab) model.closeTab(tab.id);
                            }}
                        >
                            Close tab
                        </button>
                    </div>
                </Show>

                <Show when={model.errorAtom() && model.activeTabAtom()?.contentLoaded}>
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
                    when={
                        model.filePathAtom() &&
                        !model.loadingAtom() &&
                        // Don't render CodeMirror when the active tab failed
                        // to load — the centered error panel takes the body.
                        !(model.errorAtom() && model.activeTabAtom() && !model.activeTabAtom()?.contentLoaded)
                    }
                    fallback={<EmptyEditor />}
                >
                    <div class="editor-body-wrap">
                        <div
                            class="editor-codemirror"
                            ref={setContainerRef}
                            style={{
                                display: isMarkdown() && model.editorMode() === "preview" ? "none" : undefined,
                            }}
                        />
                        <Show when={isMarkdown() && model.editorMode() === "split"}>
                            <div
                                class="editor-preview-divider"
                                onMouseDown={handlePreviewResizeMouseDown}
                                title="Drag to resize preview"
                            />
                        </Show>
                        <Show when={isMarkdown() && model.editorMode() !== "source"}>
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
                                        textAtom={() => liveDoc()}
                                        contentClass="editor-preview-markdown-content"
                                        nativeScrollbar
                                    />
                                </div>
                            </div>
                        </Show>
                    </div>
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

            {/* File-tree context menu — rendered via Portal above everything */}
            <Show when={ctxMenu()}>
                {(menu) => (
                    <ContextMenu
                        items={menu().items}
                        x={menu().x}
                        y={menu().y}
                        onClose={() => setCtxMenu(null)}
                    />
                )}
            </Show>

            {/* Delete confirmation modal — replaces window.confirm() */}
            <Show when={deleteConfirm()}>
                {(dc) => (
                    <ConfirmDialog
                        title={dc().title}
                        message={dc().message}
                        confirmLabel="Delete"
                        onConfirm={() => { dc().onConfirm(); setDeleteConfirm(null); }}
                        onCancel={() => setDeleteConfirm(null)}
                    />
                )}
            </Show>
        </div>
    );
}
