// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The keyboard-shortcuts reference (docs/keybindings.md, and the docs site's
// page), generated from the shortcut table so it can't drift from the keys.

import { helpSections } from "./help";
import { formatKey } from "./keys";
import { gestureLabels, TIPS, type TipRow } from "./tips";

/** A tip's gesture for one platform, or "—" where it doesn't apply. */
function tipGesture(tip: TipRow, platform: "mac" | "other", applies: boolean): string {
    if (!applies) return "—";
    return gestureLabels(tip, platform)
        .map((alt) => (alt[0] === "hold" ? `hold ${alt.slice(1).join(" + ")}` : alt.join(" + ")))
        .join(" or ");
}

/** A tip's text, with each `{key:…}` shown as "⌘Z / Ctrl+Z" where the platforms differ. */
function tipText(tip: TipRow): string {
    return tip.label.replace(/\{key:([^}]+)\}/g, (_m, spec: string) => {
        const mac = formatKey(spec, "mac");
        const other = formatKey(spec, "other");
        return mac === other ? mac : `${mac} / ${other}`;
    });
}

export function keybindingsDoc(): string {
    const mac = helpSections("mac");
    const other = helpSections("other");
    const linux = helpSections("linux");
    const out: string[] = [
        "# Keyboard shortcuts",
        "",
        "**Status:** living — generated from the shortcut table; it changes when the table does.",
        "",
        "<!-- Generated from frontend/app/keybindings/defaults.ts by keybindings/doc.test.ts. Do not edit by hand: run `UPDATE_KEYBINDINGS_DOC=1 npx vitest run frontend/app/keybindings/doc.test.ts`. -->",
        "",
        "The same list is in the app: press F1, or open the Help pane. A terminal keeps every key for the shell except the window, tab and pane shortcuts below, and on Windows and Linux no global shortcut uses Alt+letter, so in a terminal those always reach the shell.",
        "",
    ];
    const categories = [...new Set([...mac, ...other].map((s) => s.category))];
    for (const category of categories) {
        const m = mac.find((s) => s.category === category)?.entries ?? [];
        const o = other.find((s) => s.category === category)?.entries ?? [];
        const l = linux.find((s) => s.category === category)?.entries ?? [];
        const labels = [...new Set([...m, ...o].map((e) => e.label))];
        out.push(`## ${category}`, "", "| Action | macOS | Windows / Linux |", "|---|---|---|");
        for (const label of labels) {
            const keys = (list: typeof m) => list.find((e) => e.label === label)?.keys.join(", ") || "—";
            // Linux's own keys, where its desktop takes the shared ones.
            const both = keys(o) === keys(l) ? keys(o) : `Windows: ${keys(o)}; Linux: ${keys(l)}`;
            out.push(`| ${label} | ${keys(m)} | ${both} |`);
        }
        out.push("");
    }
    out.push(
        "## Mouse and gestures",
        "",
        "Gestures that aren't plain keys: modifier + mouse, double- and middle-click, drag and drop, and keys a pane handles itself. The Help pane shows the same list, under Mouse and gestures. Generated from frontend/app/keybindings/tips.ts, where each row is tied to the code it describes.",
        ""
    );
    const areas = [...new Set(TIPS.map((t) => t.area))];
    for (const area of areas) {
        out.push(`### ${area}`, "", "| What it does | Where | macOS | Windows / Linux |", "|---|---|---|---|");
        for (const tip of TIPS.filter((t) => t.area === area)) {
            const os = tip.os ?? ["win32", "darwin", "linux"];
            const pc = os.includes("win32") && os.includes("linux") ? "" : os.includes("win32") ? " (Windows only)" : os.includes("linux") ? " (Linux only)" : "";
            const pcApplies = os.includes("win32") || os.includes("linux");
            out.push(`| ${tipText(tip)} | ${tip.where ?? ""} | ${tipGesture(tip, "mac", os.includes("darwin"))} | ${tipGesture(tip, "other", pcApplies)}${pcApplies ? pc : ""} |`);
        }
        out.push("");
    }
    out.push(
        "## Your own shortcuts",
        "",
        "Add a `keybindings` list to your settings file (command palette → Open Settings File). Your entries come before the defaults, so they win, and they apply as soon as you save.",
        "",
        "```json",
        '"keybindings": [',
        '    { "key": "ctrl+shift+e", "command": "split:right" },',
        '    { "command": "-tab:new" },',
        '    { "key": "ctrl+Tab", "command": "-tab:next" },',
        '    { "key": "meta+k", "command": "term:clear", "platform": "mac", "when": "terminalFocus" }',
        "]",
        "```",
        "",
        "- `key`: modifiers `ctrl`, `shift`, `alt`, `meta` and `mod` (⌘ on macOS, Ctrl elsewhere), then a key: a letter, a digit, punctuation, or a name such as `Enter`, `Tab`, `ArrowLeft`, `F6`, `PageUp`. Two keys separated by a space make a chord.",
        "- `command`: the command to run (the ids are in the help pane's list and the command palette). Prefix it with `-` to unbind it: every key, or just the `key` you give. An unbind removes the defaults and your own entries above it, so put a new key for the same command after it.",
        "- `when` (optional): `textInputFocus`, `terminalFocus`, `docTabsHost`, `viewType == <pane>` or `viewType != <pane>`, each optionally negated with `!`, joined with `&&`.",
        "- `platform` (optional): `mac` or `other` (Windows and Linux). Both when omitted.",
        "",
        "An entry that can't be used (an unknown command, a bad key or `when`) is skipped and the rest still apply.",
        "",
        "In a terminal, a key you add applies when its command is one of the terminal's shortcuts above, when the command isn't in the tables above, or when its `when` includes `terminalFocus`; otherwise the terminal keeps the key for the shell.",
        "",
        "While a Browser pane's page has focus, AgentMux forwards only the default window, tab and pane keys. Remapping one of those to another command is respected there. Unbinding one stops it running, but the page still doesn't get the key; a key you add yourself isn't forwarded from a web page.",
        ""
    );
    return out.join("\n");
}
