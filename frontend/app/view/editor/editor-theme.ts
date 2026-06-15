// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CodeMirror chrome theme bound to the app's global theme tokens
// (frontend/app/theme.scss + frontend/app/themes/*.scss). The editor follows
// whatever theme the user has active instead of a fixed dark palette.
//
// Syntax-token colors come from oneDark's highlight style: the app theme
// system has no per-token color tokens, and oneDark's palette reads well on
// every (dark) app theme. Only the chrome — background, text, gutters,
// cursor, selection — is rebound to CSS variables here.

import { EditorView } from "@codemirror/view";
import { syntaxHighlighting } from "@codemirror/language";
import { oneDarkHighlightStyle } from "@codemirror/theme-one-dark";
import type { Extension } from "@codemirror/state";

const editorChromeTheme = EditorView.theme(
    {
        "&": {
            color: "var(--main-text-color)",
            // Transparent so the pane's themed background (and window
            // translucency) shows through the editor body.
            backgroundColor: "transparent",
        },
        ".cm-content": {
            caretColor: "var(--accent-color)",
        },
        ".cm-cursor, .cm-dropCursor": {
            borderLeftColor: "var(--accent-color)",
        },
        "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
            {
                backgroundColor: "var(--highlight-bg-color)",
            },
        ".cm-activeLine": {
            backgroundColor: "rgba(255, 255, 255, 0.035)",
        },
        ".cm-gutters": {
            backgroundColor: "transparent",
            color: "var(--secondary-text-color)",
            border: "none",
        },
        ".cm-activeLineGutter": {
            backgroundColor: "rgba(255, 255, 255, 0.035)",
            color: "var(--main-text-color)",
        },
        ".cm-foldPlaceholder": {
            backgroundColor: "transparent",
            border: "none",
            color: "var(--secondary-text-color)",
        },
        ".cm-panels": {
            backgroundColor: "var(--block-bg-solid-color)",
            color: "var(--main-text-color)",
        },
        ".cm-searchMatch": {
            backgroundColor: "rgba(var(--accent-color-rgb, 65, 159, 224), 0.25)",
            outline: "1px solid var(--border-color)",
        },
        ".cm-searchMatch.cm-searchMatch-selected": {
            backgroundColor: "rgba(var(--accent-color-rgb, 65, 159, 224), 0.45)",
        },
        ".cm-tooltip": {
            backgroundColor: "var(--modal-bg-color)",
            border: "1px solid var(--border-color)",
            color: "var(--main-text-color)",
        },
    },
    { dark: true },
);

export const editorTheme: Extension = [editorChromeTheme, syntaxHighlighting(oneDarkHighlightStyle)];
