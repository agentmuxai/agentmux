#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-rust-pin.mjs: the Rust version is named in rust-toolchain.toml (`channel`), and every place that installs
// Rust must name the same one: each `dtolnay/rust-toolchain@<version>` in the workflows (never `@stable`, which
// moves under us), and the `ARG RUST_VERSION` default of docker/Dockerfile.agent-developer.
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

/** What `text` (a workflow) installs: each `dtolnay/rust-toolchain@<ref>`, with its line, comments skipped. */
export function workflowRefs(text) {
    const out = [];
    text.split("\n").forEach((line, i) => {
        if (/^\s*#/.test(line)) return;
        const m = /uses:\s*dtolnay\/rust-toolchain@(\S+)/.exec(line);
        if (m) out.push({ line: i + 1, ref: m[1] });
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

/** Everything that disagrees with `channel`, as lines to print. */
export function problems(channel, workflows, dockerfile) {
    const out = [];
    if (!channel || !/^\d+\.\d+\.\d+$/.test(channel)) {
        out.push(`rust-toolchain.toml: channel must be a full version like 1.99.1, not ${JSON.stringify(channel)} (a channel such as "stable" is what this pins against)`);
        return out;
    }
    for (const [file, text] of Object.entries(workflows)) {
        for (const { line, ref } of workflowRefs(text)) {
            if (ref !== channel) out.push(`${file}:${line}: dtolnay/rust-toolchain@${ref} should be @${channel}`);
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
    const found = problems(pinnedChannel(readFileSync(tomlPath, "utf8")), workflows, readFileSync(join(ROOT, "docker/Dockerfile.agent-developer"), "utf8"));
    if (found.length) {
        console.error("check-rust-pin: the Rust version is not the same everywhere:\n  " + found.join("\n  "));
        process.exit(1);
    }
    console.log(`check-rust-pin: ok (Rust ${pinnedChannel(readFileSync(tomlPath, "utf8"))} in rust-toolchain.toml, the workflows and the developer image)`);
}
