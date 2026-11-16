#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-rust-pin.mjs: the Rust version is named in rust-toolchain.toml (`channel`), and every place that installs
// Rust must name the same one: each `dtolnay/rust-toolchain@<version>` in the workflows (never `@stable`, which
// moves under us), and the `ARG RUST_VERSION` default of docker/Dockerfile.agent-developer. Each of those installs
// also names the `components` rust-toolchain.toml lists: rustup adds a missing one the first time cargo runs in the
// repo, in the middle of the build, outside the install step's retries (the 0.60.30 release's Linux build failed
// there: "detected conflict: 'bin/cargo-fmt'").
// docs/specs/SPEC_CONTAINER_AGENT_DEVELOPER_IMAGE_2026_10_29.md section 2.
//
//   node scripts/check-rust-pin.mjs
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

/** The `channel` of a rust-toolchain.toml, or null. */
export function pinnedChannel(toml) {
    const m = /^\s*channel\s*=\s*"([^"]+)"/m.exec(toml);
    return m ? m[1] : null;
}

/** The `components` of a rust-toolchain.toml: the names, sorted, or an empty list. */
export function pinnedComponents(toml) {
    const m = /^\s*components\s*=\s*\[([^\]]*)\]/m.exec(toml);
    return m ? [...m[1].matchAll(/"([^"]+)"/g)].map((x) => x[1]).sort() : [];
}

/** The components a step names in its `with: components:` (commas or spaces), sorted. `lines` is the step's own lines. */
function stepComponents(lines) {
    for (const line of lines) {
        const m = /^\s*components:\s*(.*?)\s*(?:#.*)?$/.exec(line);
        if (m) return m[1].split(/[\s,]+/).filter(Boolean).sort();
    }
    return [];
}

/** What `text` (a workflow) installs: each `dtolnay/rust-toolchain@<ref>`, with its line and the components it asks for, comments skipped. */
export function workflowRefs(text) {
    const out = [];
    const lines = text.split("\n");
    lines.forEach((line, i) => {
        if (/^\s*#/.test(line)) return;
        const m = /^(\s*)-\s*uses:\s*dtolnay\/rust-toolchain@(\S+)/.exec(line) ?? /^(\s*)uses:\s*dtolnay\/rust-toolchain@(\S+)/.exec(line);
        if (!m) return;
        // The step runs to the next line that starts a sibling step or ends the list.
        const indent = m[1].length;
        const own = [];
        for (const next of lines.slice(i + 1)) {
            if (next.trim() === "") continue;
            const nextIndent = next.length - next.trimStart().length;
            if (nextIndent < indent || (nextIndent === indent && /^\s*-\s/.test(next))) break;
            own.push(next);
        }
        out.push({ line: i + 1, ref: m[2], components: stepComponents(own) });
    });
    return out;
}

/** Every workflow and composite action under `dir` (.yml or .yaml in workflows/, action.yml or action.yaml in actions/). */
export function installerFiles(dir) {
    const out = [];
    const walk = (d, inActions) => {
        if (!existsSync(d)) return;
        for (const name of readdirSync(d)) {
            const full = join(d, name);
            if (statSync(full).isDirectory()) walk(full, inActions || name === "actions");
            else if (/\.ya?ml$/.test(name) && (d.endsWith("workflows") || (inActions && /^action\.ya?ml$/.test(name)))) out.push(full);
        }
    };
    walk(dir, false);
    return out;
}

/** The default of `ARG RUST_VERSION=` in a Dockerfile, or null. */
export function dockerfileVersion(text) {
    const m = /^ARG\s+RUST_VERSION=(\S+)/m.exec(text);
    return m ? m[1] : null;
}

/** Everything that disagrees with `channel` (and, when `components` is given, with its components), as lines to print. */
export function problems(channel, workflows, dockerfile, components) {
    const out = [];
    if (!channel || !/^\d+\.\d+\.\d+$/.test(channel)) {
        out.push(`rust-toolchain.toml: channel must be a full version like 1.99.1, not ${JSON.stringify(channel)} (a channel such as "stable" is what this pins against)`);
        return out;
    }
    for (const [file, text] of Object.entries(workflows)) {
        for (const { line, ref, components: asked } of workflowRefs(text)) {
            if (ref !== channel) out.push(`${file}:${line}: dtolnay/rust-toolchain@${ref} should be @${channel}`);
            if (components && asked.join(",") !== components.join(",")) {
                out.push(`${file}:${line}: dtolnay/rust-toolchain@${ref} should install the components rust-toolchain.toml lists (with: components: ${components.join(", ")}), not ${asked.length ? asked.join(", ") : "none"}`);
            }
        }
    }
    const v = dockerfileVersion(dockerfile);
    if (v !== channel) out.push(`docker/Dockerfile.agent-developer: ARG RUST_VERSION=${v} should be ${channel}`);
    return out;
}

if (isMain(import.meta.url)) {
    const tomlPath = join(ROOT, "rust-toolchain.toml");
    if (!existsSync(tomlPath)) {
        console.error("check-rust-pin: rust-toolchain.toml is missing");
        process.exit(1);
    }
    const workflows = {};
    for (const file of installerFiles(join(ROOT, ".github"))) workflows[relative(ROOT, file)] = readFileSync(file, "utf8");
    const toml = readFileSync(tomlPath, "utf8");
    const found = problems(pinnedChannel(toml), workflows, readFileSync(join(ROOT, "docker/Dockerfile.agent-developer"), "utf8"), pinnedComponents(toml));
    if (found.length) {
        console.error("check-rust-pin: the Rust version is not the same everywhere:\n  " + found.join("\n  "));
        process.exit(1);
    }
    console.log(`check-rust-pin: ok (Rust ${pinnedChannel(toml)} in rust-toolchain.toml, the workflows and the developer image; components ${pinnedComponents(toml).join(", ") || "none"} in every workflow install)`);
}
