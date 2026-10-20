// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CodeMirror chrome theme bound to the app's global theme tokens
// (frontend/app/theme.scss + frontend/app/themes/*.scss). The editor follows
// whatever theme the user has active instead of a fixed dark palette.
//
// Syntax-token colours are the `--syntax-*` tokens (theme.scss): One Dark's
// palette on the dark themes and a palette checked for contrast on the light
// ones, switched by the `data-theme-polarity` marker, so a theme change
// recolours the code with no reconfiguring. The chrome — background, text,
// gutters, cursor, selection — is rebound to CSS variables here. Only
// CodeMirror's own light/dark base styles need the polarity told to them:
// `editorTheme(light)` sets that, and the editor swaps it in a Compartment.

import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import type { Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { tags } from "@lezer/highlight";

const editorChromeTheme = (dark: boolean) =>
    EditorView.theme(
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
                backgroundColor: "color-mix(in srgb, var(--accent-color) 10%, transparent)",
            },
            ".cm-gutters": {
                backgroundColor: "transparent",
                // Muted accent shade — dimmer than the active line's number
                // (below) so the current line's number visibly stands out
                // instead of blending into every other line number.
                color: "color-mix(in srgb, var(--accent-color) 55%, var(--secondary-text-color))",
                border: "none",
            },
            ".cm-activeLineGutter": {
                backgroundColor: "color-mix(in srgb, var(--accent-color) 10%, transparent)",
                // Full-strength accent — the distinct, brighter shade for the
                // current line's number.
                color: "var(--accent-color)",
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

            // ── Find/replace panel ──────────────────────────────────────────
            // Tight spacing + hard corners (app convention) + accent woven in via
            // color-mix, mirroring the Accounts gallery (accounts-gallery.scss:
            // accent-mixed borders/hover, accent toggles). theme.scss tokens only.
            ".cm-panels.cm-panels-top": {
                borderBottom: "1px solid color-mix(in srgb, var(--accent-color) 30%, var(--border-color))",
            },
            ".cm-panel.cm-search": {
                position: "relative",
                display: "flex",
                flexWrap: "wrap",
                alignItems: "center",
                gap: "3px",
                padding: "4px 26px 4px 6px",
                fontFamily: "inherit",
                fontSize: "12px",
            },
            ".cm-panel.cm-search label": {
                display: "inline-flex",
                alignItems: "center",
                gap: "3px",
                margin: "0 0 0 2px",
                color: "var(--secondary-text-color)",
                fontSize: "11px",
                textTransform: "none",
                cursor: "pointer",
            },
            ".cm-panel.cm-search input[type=checkbox]": {
                accentColor: "var(--accent-color)",
                margin: "0",
                cursor: "pointer",
            },
            ".cm-textfield": {
                backgroundColor: "var(--form-element-bg-color)",
                color: "var(--main-text-color)",
                border: "1px solid var(--border-color)",
                borderRadius: "0", // hard corners — app convention
                margin: "0",
                padding: "2px 6px",
                fontSize: "12px",
                fontFamily: "inherit",
            },
            ".cm-textfield:focus": {
                outline: "none",
                borderColor: "var(--accent-color)",
            },
            // Accent-tinted buttons — hard corners, theme accent via color-mix
            // (same technique as the Accounts tiles), brighter on hover.
            ".cm-button": {
                backgroundColor: "color-mix(in srgb, var(--accent-color) 12%, transparent)",
                backgroundImage: "none",
                color: "var(--main-text-color)",
                border: "1px solid color-mix(in srgb, var(--accent-color) 35%, var(--border-color))",
                borderRadius: "0",
                margin: "0",
                padding: "2px 7px",
                fontSize: "11px",
                cursor: "pointer",
            },
            ".cm-button:hover": {
                backgroundColor: "color-mix(in srgb, var(--accent-color) 26%, transparent)",
                borderColor: "var(--accent-color)",
                color: "var(--main-text-color)",
            },
            ".cm-button:active": {
                backgroundColor: "color-mix(in srgb, var(--accent-color) 38%, transparent)",
            },
            ".cm-search button[name=close]": {
                position: "absolute",
                top: "4px",
                right: "6px",
                padding: "0 3px",
                margin: "0",
                border: "none",
                background: "transparent",
                color: "var(--secondary-text-color)",
                fontSize: "15px",
                lineHeight: "1",
                cursor: "pointer",
            },
            ".cm-search button[name=close]:hover": {
                color: "var(--accent-color)",
                backgroundColor: "transparent",
            },
        },
        { dark }
    );

const v = (name: string) => `var(--syntax-${name})`;

// One Dark's tag-to-colour mapping (@codemirror/theme-one-dark), with the
// colours as tokens.
const syntaxStyle = HighlightStyle.define([
    { tag: tags.keyword, color: v("keyword") },
    { tag: [tags.name, tags.deleted, tags.character, tags.propertyName, tags.macroName], color: v("name") },
    { tag: [tags.function(tags.variableName), tags.labelName], color: v("function") },
    { tag: [tags.color, tags.constant(tags.name), tags.standard(tags.name)], color: v("constant") },
    { tag: [tags.definition(tags.name), tags.separator], color: v("plain") },
    {
        tag: [
            tags.typeName,
            tags.className,
            tags.number,
            tags.changed,
            tags.annotation,
            tags.modifier,
            tags.self,
            tags.namespace,
        ],
        color: v("type"),
    },
    {
        tag: [
            tags.operator,
            tags.operatorKeyword,
            tags.url,
            tags.escape,
            tags.regexp,
            tags.link,
            tags.special(tags.string),
        ],
        color: v("operator"),
    },
    { tag: [tags.meta, tags.comment], color: v("comment") },
    { tag: tags.strong, fontWeight: "bold" },
    { tag: tags.emphasis, fontStyle: "italic" },
    { tag: tags.strikethrough, textDecoration: "line-through" },
    { tag: tags.link, color: v("comment"), textDecoration: "underline" },
    { tag: tags.heading, fontWeight: "bold", color: v("name") },
    { tag: [tags.atom, tags.bool, tags.special(tags.variableName)], color: v("constant") },
    { tag: [tags.processingInstruction, tags.string, tags.inserted], color: v("string") },
    { tag: tags.invalid, color: v("invalid") },
]);

/** The editor's theme for a light or a dark app theme. */
export const editorTheme = (light: boolean): Extension => [editorChromeTheme(!light), syntaxHighlighting(syntaxStyle)];

/** Whether the app's theme is a light one (the marker app.tsx keeps on <html>). */
export const isLightPolarity = (): boolean => document.documentElement.getAttribute("data-theme-polarity") === "light";
