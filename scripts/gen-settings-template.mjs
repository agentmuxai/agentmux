#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// gen-settings-template.mjs - writes settings-template.jsonc from schema/settings.json, so the file a new
// install starts from lists every setting once, grouped, with its description and default commented out.
// Spec: docs/specs/SPEC_SETTINGS_JSON_EVERY_KEY_ONE_TO_ONE_2026_11_01.md section 5.1, 5.2.
//
//   node scripts/gen-settings-template.mjs           write settings-template.jsonc
//   node scripts/gen-settings-template.mjs --check   fail if it is not what this would write (CI)
//
// A key's block is its description, a note for what is special about it (Windows only, a secret), and its
// commented default. A key with no default but an `x-example` shows the example and says so; one with
// neither is listed at the end by name. Families (`app:*`, `widget:hidden@<widget>`) are left out.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCHEMA = path.join(root, "schema", "settings.json");
const OUT = path.join(root, "settings-template.jsonc");

/** The order the groups appear in; a section not listed here fails, so a new one is placed on purpose. */
export const SECTION_ORDER = [
    "Terminal", "Drag and drop", "Animations", "Window", "Panes and tabs", "Keyboard", "Shell environment", "App",
    "Telemetry", "Connections", "Notifications", "Voice input", "Attachments", "Agents", "Swarm supervisor",
    "Kanban", "Browser panes", "Jobs", "Desktop control", "Files pane", "Network", "Cloud and sharing", "Messaging",
];

const WRAP = 96;
const isFamily = (key) => key.includes("*") || key.includes("@") || key.includes("<");
const PLATFORM = { windows: "Windows", macos: "macOS", linux: "Linux" };

/** `text` as `// ` lines under `indent`, broken at word ends before WRAP columns. */
function commentLines(text, indent) {
    const out = [];
    for (const paragraph of text.split("\n")) {
        let line = "";
        for (const word of paragraph.split(/\s+/).filter(Boolean)) {
            if (line && indent.length + 3 + line.length + 1 + word.length > WRAP) {
                out.push(`${indent}// ${line}`);
                line = word;
            } else {
                line = line ? `${line} ${word}` : word;
            }
        }
        if (line) out.push(`${indent}// ${line}`);
    }
    return out;
}

export function generate(schema) {
    const props = schema.$defs.SettingsType.properties;
    const bySection = new Map();
    for (const [key, def] of Object.entries(props)) {
        if (isFamily(key)) continue;
        const section = def["x-section"];
        if (!section) throw new Error(`${key} has no x-section`);
        if (!SECTION_ORDER.includes(section)) throw new Error(`${key}: section "${section}" is not in SECTION_ORDER`);
        if (!bySection.has(section)) bySection.set(section, []);
        bySection.get(section).push([key, def]);
    }

    const out = [
        "// AgentMux settings. Saved changes apply at once.",
        "// Every setting is listed with its default. Remove the leading // to override one.",
        "// Docs: https://docs.agentmux.ai/settings",
        "// This file is generated from schema/settings.json (node scripts/gen-settings-template.mjs).",
        "{",
    ];
    const undefaulted = [];
    for (const section of SECTION_ORDER) {
        const entries = bySection.get(section);
        if (!entries) continue;
        out.push(`    // -- ${section} --`, "");
        for (const [key, def] of entries) {
            const secret = def["x-secret"] === true;
            const hasDefault = "default" in def;
            const hasExample = "x-example" in def;
            if (!hasDefault && !hasExample && !secret) {
                undefaulted.push([key, def]);
                continue;
            }
            const notes = [];
            const platform = def["x-platform"];
            if (platform) notes.push(`[${platform.map((p) => PLATFORM[p] ?? p).join(" and ")} only]`);
            const text = [notes.join(" "), def.description ?? ""].filter(Boolean).join(" ");
            const block = [];
            if (text) block.push(...commentLines(text, "    "));
            if (secret) block.push(...commentLines("Secret: stored here as typed. Never paste this file into a chat or a ticket without removing it.", "    "));
            out.push(...block);
            let value;
            let tail = "";
            if (secret) value = '""';
            else if (hasDefault) value = JSON.stringify(def.default);
            else {
                value = JSON.stringify(def["x-example"]);
                tail = `  // example; ${def["x-nodefault"] ?? "no default"}`;
            }
            // A key with a note is set off by a blank line; bare keys stay together.
            out.push(`    // ${JSON.stringify(key)}: ${value},${tail}`);
            if (block.length) out.push("");
        }
        if (out[out.length - 1] !== "") out.push("");
    }
    if (undefaulted.length) {
        out.push("    // -- Without a default --", "    // These settings have no value to show: unset means the app decides.", "");
        for (const [key, def] of undefaulted) {
            const text = [`${key}:`, def.description ?? "(no description yet)"].join(" ");
            out.push(...commentLines(text, "    "));
        }
        out.push("");
    }
    // Drop the blank line before the closing brace.
    while (out[out.length - 1] === "") out.pop();
    out.push("}", "");
    return out.join("\n");
}

function main() {
    const check = process.argv.includes("--check");
    const schema = JSON.parse(fs.readFileSync(SCHEMA, "utf8"));
    const text = generate(schema);
    if (check) {
        const have = fs.existsSync(OUT) ? fs.readFileSync(OUT, "utf8") : "";
        if (have !== text) {
            console.error("settings-template.jsonc is not what scripts/gen-settings-template.mjs writes from schema/settings.json.");
            console.error("Run: node scripts/gen-settings-template.mjs   and commit the file.");
            process.exit(1);
        }
        console.log("gen-settings-template: ok");
        return;
    }
    fs.writeFileSync(OUT, text);
    console.log(`gen-settings-template: wrote ${path.relative(root, OUT)}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
