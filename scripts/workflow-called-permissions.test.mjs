// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A reusable workflow's jobs can't hold more permission than the job that calls it grants, however deep the
// chain. actionlint passes a call that asks for more, and GitHub then refuses the whole run at startup: the
// v0.60.8 Release built nothing because a called job asked for `issues: write` and release.yml's `release` job
// granted contents, id-token and packages. This walks every `uses: …/.github/workflows/x.yml@ref` call and
// fails when a job of the called file (or the file's own top-level grant, for a job with none) is above what
// the caller grants. A caller with no `permissions` of its own is skipped: its grant is the repository default.
//
// The workflows are read by indentation (the keys this needs are all plain), as workflow-sparse-checkout.test.mjs
// does, so the test needs no YAML package.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const WORKFLOWS = join(dirname(fileURLToPath(import.meta.url)), "..", ".github/workflows");
const LEVEL = { none: 0, read: 1, write: 2 };

/** The text after `key:` on a line, without a trailing comment or quotes. */
const value = (rest) => rest.replace(/\s+#.*$/, "").trim().replace(/^["']|["']$/g, "");

/**
 * { permissions, jobs: { name: { permissions, uses } } } of a workflow file. `permissions` is a string
 * (`read-all`, `write-all`), a { scope: level } map, or undefined when the level has none.
 */
function load(file) {
    const lines = readFileSync(join(WORKFLOWS, file), "utf8").replace(/\r\n/g, "\n").split("\n");
    const doc = { permissions: undefined, jobs: {} };
    let section = null; // "permissions" | "jobs" at the top level
    let job = null; // the job being read
    let perms = null; // the permissions block being read: its key's indent and its map
    const permissionsOf = (v) => {
        if (v === "{}") return {};
        if (v) return v;
        return perms.map;
    };
    for (const line of lines) {
        if (/^\s*(#.*)?$/.test(line)) continue;
        const indent = line.search(/\S/);
        const m = /^\s*([A-Za-z0-9_-]+):(.*)$/.exec(line);
        if (perms && indent > perms.indent && m) {
            perms.map[m[1]] = value(m[2]);
            continue;
        }
        perms = null;
        if (!m) continue;
        const key = m[1];
        const v = value(m[2]);
        if (indent === 0) {
            section = key === "permissions" || key === "jobs" ? key : null;
            job = null;
            if (key === "permissions") {
                if (!v) perms = { indent, map: {} };
                doc.permissions = permissionsOf(v);
            }
        } else if (section === "jobs" && indent === 2) {
            job = doc.jobs[key] = {};
        } else if (section === "jobs" && job && indent === 4) {
            if (key === "uses") job.uses = v;
            if (key === "permissions") {
                if (!v) perms = { indent, map: {} };
                job.permissions = permissionsOf(v);
            }
        }
    }
    return doc;
}

/** `read-all` / `write-all` strings and maps alike, as { scope: level }; null when nothing is declared. */
function grant(perms) {
    if (perms == null) return null;
    if (perms === "read-all") return { "*": "read" };
    if (perms === "write-all") return { "*": "write" };
    return perms;
}

const levelOf = (g, scope) => LEVEL[g[scope] ?? g["*"] ?? "none"];

/** The scopes of `want` that `cap` doesn't allow. */
function over(want, cap) {
    return Object.entries(want)
        .filter(([scope, level]) => scope !== "*" && LEVEL[level] > levelOf(cap, scope))
        .map(([scope, level]) => `${scope}: ${level}`);
}

/** The workflow file a `uses:` in this repo calls, or null (an action, or another repository). */
const calledFile = (uses) => /^(?:agentmuxai\/agentmux\/|\.\/)\.github\/workflows\/([\w.-]+\.ya?ml)(?:@.*)?$/.exec(uses ?? "")?.[1] ?? null;

/** Every violation reachable from `file` called with at most `cap`; `trail` names the chain. */
function violations(file, cap, trail, seen = []) {
    if (seen.includes(file)) return [];
    const doc = load(file);
    const top = grant(doc.permissions);
    const out = [];
    for (const [name, job] of Object.entries(doc.jobs ?? {})) {
        const own = grant(job.permissions) ?? top;
        const here = `${trail} > ${file} ${name}`;
        if (own) {
            const bad = over(own, cap);
            if (bad.length) out.push(`${here} asks for ${bad.join(", ")}`);
        }
        const next = calledFile(job.uses);
        if (next) out.push(...violations(next, own ? own : cap, here, [...seen, file]));
    }
    return out;
}

const callers = readdirSync(WORKFLOWS)
    .filter((f) => /\.ya?ml$/.test(f))
    .flatMap((file) => {
        const doc = load(file);
        const top = grant(doc.permissions);
        return Object.entries(doc.jobs ?? {})
            .filter(([, job]) => calledFile(job.uses))
            .map(([name, job]) => ({ id: `${file} ${name}`, cap: grant(job.permissions) ?? top, file: calledFile(job.uses) }));
    })
    .filter((c) => c.cap);

describe("a called workflow asks for no more than its caller grants", () => {
    it.each(callers.map((c) => [c.id, c]))("%s", (id, c) => {
        expect(violations(c.file, c.cap, id)).toEqual([]);
    });

    it("finds the release chain it is meant to guard", () => {
        expect(callers.map((c) => c.id)).toEqual(
            expect.arrayContaining(["release.yml release", "release-core.yml linux-upgrade-check"]),
        );
    });

    it("sees a call that asks for more than it is given", () => {
        const cap = { contents: "write", "id-token": "write", packages: "write" };
        expect(over({ contents: "read", issues: "write" }, cap)).toEqual(["issues: write"]);
        expect(over({ contents: "read", issues: "read" }, cap)).toEqual(["issues: read"]);
        expect(over({ contents: "read" }, cap)).toEqual([]);
    });
});
