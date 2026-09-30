// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { Blockquote, Paragraph, Root } from "mdast";
import type { Plugin } from "unified";
import { visit } from "unist-util-visit";

/**
 * GitHub's alert kinds (SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §3). Three
 * places must agree with this list, and `markdown-callout.test.tsx` checks all
 * three: the sanitizer allowlist (`markdown.tsx`), the stylesheet
 * (`markdown.scss`), and the Operator Config entry that tells agents about it
 * (`agentmux-srv/operator-config-seed.json`, id `operator-config-rich-output`).
 */
export const ALERT_KINDS = ["note", "tip", "important", "warning", "caution"] as const;
export type AlertKind = (typeof ALERT_KINDS)[number];

/** Every class the plugin emits, for the sanitizer's `div` allowlist. */
export const ALERT_CLASSES = [
    "markdown-alert",
    "markdown-alert-title",
    ...ALERT_KINDS.map((k) => `markdown-alert-${k}`),
];

const TITLES: Record<AlertKind, string> = {
    note: "Note",
    tip: "Tip",
    important: "Important",
    warning: "Warning",
    caution: "Caution",
};

/** `[!KIND]` alone on the first line: followed by a line break or nothing. */
const MARKER = /^\[!([A-Za-z]+)\][ \t]*(\n|$)/;

const isKind = (k: string): k is AlertKind => (ALERT_KINDS as readonly string[]).includes(k);

/**
 * Turns a blockquote whose first line is exactly `[!NOTE]` (or another GitHub
 * kind, any case) into `<div class="markdown-alert markdown-alert-note">` with
 * a title row, the same shape GitHub renders. Anything else — an unknown kind,
 * a marker with text after it on the same line, a marker mid-sentence — stays
 * an ordinary blockquote, so text written for GitHub renders the same here.
 */
const remarkGithubAlerts: Plugin<[], Root> = function () {
    return (tree: Root) => {
        visit(tree, "blockquote", (node: Blockquote) => {
            const first = node.children[0];
            if (first?.type !== "paragraph") return;
            const lead = first.children[0];
            if (lead?.type !== "text") return;
            const m = MARKER.exec(lead.value);
            if (!m) return;
            const kind = m[1].toLowerCase();
            if (!isKind(kind)) return;

            // The marker's text node ended without a newline: the line must
            // end there too, i.e. a hard break or nothing else follows.
            if (m[2] === "") {
                const next = first.children[1];
                if (next && next.type !== "break") return;
                if (next) first.children.splice(1, 1);
            }

            const rest = lead.value.slice(m[0].length);
            if (rest) lead.value = rest;
            else first.children.shift();
            if (first.children.length === 0) node.children.shift();

            const title: Paragraph = {
                type: "paragraph",
                data: { hName: "div", hProperties: { className: ["markdown-alert-title"] } },
                children: [{ type: "text", value: TITLES[kind] }],
            };
            node.children.unshift(title);
            node.data = {
                ...node.data,
                hName: "div",
                hProperties: { className: ["markdown-alert", `markdown-alert-${kind}`] },
            };
        });
    };
};

export default remarkGithubAlerts;
