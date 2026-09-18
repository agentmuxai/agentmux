#!/usr/bin/env node
// Four checks on the Rust -> TypeScript RPC codegen, each written because the
// same defect got through review more than once.
//
// None of these is covered by an existing gate, and that is the point:
//
//   * `cargo check` sees only the Rust half.
//   * `tsc` checks the frontend against the GENERATED types, never against the
//     backend, so a binding that describes a wire format nobody speaks still
//     typechecks cleanly on both sides.
//   * `check-rpc-bindings.sh` checks that a generated file EXISTS per type and
//     is current with the Rust. A generated file can be perfectly current and
//     still be wrong about the wire.
//
// Every failure mode below therefore compiles, typechecks, and passes the
// bindings gate, while being broken at run time. Run from the repo root.
//
// Background: docs/reports/REPORT_MIGRATION_WRAPUP_STATUS_2026_09_16.md §5.4e.

import fs from "node:fs";
import path from "node:path";

const failures = [];
const note = (msg) => console.log(msg);

function walk(dir, ext, out = []) {
    if (!fs.existsSync(dir)) return out;
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        const f = path.join(dir, e.name);
        if (e.isDirectory()) walk(f, ext, out);
        else if (e.name.endsWith(ext)) out.push(f);
    }
    return out;
}

const rel = (f) => path.relative(".", f).replace(/\\/g, "/");

// ---------------------------------------------------------------------------
// 1. Ambient globals that duplicate a generated type.
//
// A consumer that does not import the generated type resolves to the ambient
// one instead -- silently, because both exist and both are valid. This was the
// review finding on #3318, twice on #3320, and again on #3327.
//
// Two files declare ambient globals, not one. Scanning only srv-types.d.ts
// would have passed the editor-reads slice while leaving `DirEntry` duplicated
// in custom.d.ts -- the exact drift the check exists to catch.
// ---------------------------------------------------------------------------
function ambientDuplicates() {
    const genDir = "frontend/types/rpc";
    if (!fs.existsSync(genDir)) return;
    const generated = new Set(
        fs.readdirSync(genDir).filter((f) => f.endsWith(".ts")).map((f) => f.replace(/\.ts$/, ""))
    );

    // Documented carve-outs, kept ambient on purpose.
    // TokenCounts -- other AMBIENT globals reference it, and an ambient
    // declaration cannot import, so it can only go once those migrate too.
    const ALLOWED = new Set(["TokenCounts"]);

    const dupes = [];
    let total = 0;
    for (const file of ["frontend/types/srv-types.d.ts", "frontend/types/custom.d.ts"]) {
        if (!fs.existsSync(file)) continue;
        const d = fs.readFileSync(file, "utf8").replace(/\r\n/g, "\n");
        for (const m of d.matchAll(/^ {4}type (\w+)\s*=/gm)) {
            total++;
            if (generated.has(m[1]) && !ALLOWED.has(m[1])) {
                dupes.push(`${rel(file)}:${d.slice(0, m.index).split("\n").length}  ${m[1]}`);
            }
        }
    }
    if (dupes.length) {
        failures.push(
            `${dupes.length} ambient global(s) duplicate a generated type -- consumers ` +
            `that do not import will silently resolve to the stale one:\n  ` + dupes.join("\n  ")
        );
    } else {
        note(`  ok  ${total} ambient globals, none duplicate a generated type`);
    }
}

// ---------------------------------------------------------------------------
// 2. ts-rs ignores per-field and per-variant `#[serde(rename = "...")]`.
//
// The field generates under its RUST name -- a key nothing sends and nothing
// reads. Seen three times: `oauth_config_dir` (#3320), `FlowNode.type` ->
// `node_type` (#3329), `pathSource` -> `path_source` (#3344).
//
// `serde(rename_all)` is NOT checked here, on purpose: ts-rs DOES honour it, on
// structs and enums both. An earlier version of this check matched it too and
// reported 44 false positives -- investigating those is what disproved the
// claim that it was ignored. See #3345.
// ---------------------------------------------------------------------------
function serdeRenameWithoutTsRename() {
    const SERDE = /#\[serde\([^\]]*\brename\s*=/;
    const TS = /#\[ts\([^\]]*\brename\s*=/;
    const hits = [];

    for (const f of walk("agentmux-srv/src", ".rs")) {
        const lines = fs.readFileSync(f, "utf8").replace(/\r\n/g, "\n").split("\n");
        for (let i = 0; i < lines.length; i++) {
            if (!/^#\[derive\(.*\bts_rs::TS\b.*\)\]/.test(lines[i])) continue;
            let end = i + 1, opened = false;
            for (; end < lines.length; end++) {
                if (!opened && /[{;]\s*$/.test(lines[end])) {
                    if (/;\s*$/.test(lines[end])) break;
                    opened = true;
                    continue;
                }
                if (opened && /^\}/.test(lines[end])) break;
            }
            for (let j = i; j <= end && j < lines.length; j++) {
                if (!SERDE.test(lines[j])) continue;
                // Scan the contiguous attribute/doc run around this line.
                let k = j, found = false;
                while (k > i && /^\s*#\[/.test(lines[k])) { if (TS.test(lines[k])) found = true; k--; }
                k = j;
                while (k < lines.length && /^\s*(#\[|\/\/)/.test(lines[k])) { if (TS.test(lines[k])) found = true; k++; }
                if (!found) hits.push(`${rel(f)}:${j + 1}  ${lines[j].trim()}`);
            }
        }
    }
    if (hits.length) {
        failures.push(
            `${hits.length} serde(rename) on a ts_rs::TS type with no matching ts(rename) -- ` +
            `these generate the RUST field name, which nothing sends:\n  ` + hits.join("\n  ")
        );
    } else {
        note("  ok  every serde(rename) on a generated type has a matching ts(rename)");
    }
}

// ---------------------------------------------------------------------------
// 3. A `register_typed` handler that hand-serializes its response.
//
// The schema registry records the DECLARED Resp type, so returning
// `serde_json::to_value(..)` or an inline `json!({..})` makes it record `Value`
// and leaves that endpoint outside the drift net -- while the wire bytes stay
// identical, so nothing else notices. Found by review on #3327
// (`getagentcontent`); the inline-`json!` half was a false negative in this
// check's first version, caught by a test on #3331 (`movescratchfile`).
// ---------------------------------------------------------------------------
function typedHandlersHandSerializing() {
    const hits = [];
    for (const f of walk("agentmux-srv/src/server", ".rs")) {
        const src = fs.readFileSync(f, "utf8").replace(/\r\n/g, "\n");
        let idx = 0;
        while (true) {
            const i = src.indexOf("engine.register_typed(", idx);
            if (i < 0) break;
            let next = src.indexOf("engine.register_", i + 10);
            if (next < 0) next = src.length;
            const block = src.slice(i, next);
            const cmd = (block.match(/\n\s*(COMMAND_\w+|"[\w.:\-]+")/) || [])[1] || "?";
            const re = /^\s*(?:return )?Ok\((?:Some\()?serde_json::(?:to_value|json!)|^\s*Ok\([\w.]+\.map\(\|\w+\| serde_json::to_value/gm;
            for (const m of block.matchAll(re)) {
                hits.push(`${rel(f)}:${src.slice(0, i + m.index).split("\n").length}  ${cmd}`);
            }
            idx = i + 10;
        }
    }
    if (hits.length) {
        failures.push(
            `${hits.length} typed handler(s) hand-serialize their response -- the registry ` +
            `records Value, not the real type:\n  ` + hits.join("\n  ")
        );
    } else {
        note("  ok  no typed handler hand-serializes its response");
    }
}

// ---------------------------------------------------------------------------
// 4. Imports left behind by a migration.
//
// Comments and strings are stripped before deciding whether a name is used --
// a regex that skips that step matched type names inside comments and left
// nine files with unused imports on an earlier slice.
// ---------------------------------------------------------------------------
function deadGeneratedImports() {
    const strip = (src) => {
        let out = "", i = 0;
        while (i < src.length) {
            const two = src.slice(i, i + 2);
            if (two === "//") { while (i < src.length && src[i] !== "\n") i++; continue; }
            if (two === "/*") { i += 2; while (i < src.length && src.slice(i, i + 2) !== "*/") i++; i += 2; continue; }
            const c = src[i];
            if (c === '"' || c === "'" || c === "`") {
                const q = c; i++;
                while (i < src.length && src[i] !== q) { if (src[i] === "\\") i++; i++; }
                i++; out += " "; continue;
            }
            out += c; i++;
        }
        return out;
    };

    const hits = [];
    for (const f of walk("frontend/app", ".ts").concat(walk("frontend/app", ".tsx"))) {
        const src = fs.readFileSync(f, "utf8");
        const body = strip(src);
        for (const m of src.matchAll(/^import type \{ ([^}]+) \} from "@\/types\/rpc\/[^"]+";$/gm)) {
            for (const spec of m[1].split(",").map((x) => x.trim()).filter(Boolean)) {
                // `X as XT` binds XT, not X -- several stub files alias to dodge
                // a name clash with an ambient global. Checking the left-hand
                // name reports every one of them as unused.
                const local = (spec.split(/\s+as\s+/)[1] || spec).trim();
                if (!/^\w+$/.test(local)) continue;
                // One occurrence is the import itself; anything real uses it again.
                const uses = (body.match(new RegExp(`\\b${local}\\b`, "g")) || []).length;
                if (uses <= 1) hits.push(`${rel(f)}  ${spec}`);
            }
        }
    }
    if (hits.length) {
        failures.push(`${hits.length} unused generated import(s):\n  ` + hits.join("\n  "));
    } else {
        note("  ok  no unused generated imports");
    }
}

console.log("check-rpc-codegen-hygiene:");
ambientDuplicates();
serdeRenameWithoutTsRename();
typedHandlersHandSerializing();
deadGeneratedImports();

if (failures.length) {
    console.error("\ncheck-rpc-codegen-hygiene: FAILED\n");
    for (const f of failures) console.error(f + "\n");
    console.error(
        "None of these break `cargo check`, `tsc`, or check-rpc-bindings.sh -- that is why\n" +
        "this check exists. See docs/reports/REPORT_MIGRATION_WRAPUP_STATUS_2026_09_16.md §5.4e.\n"
    );
    process.exit(1);
}
console.log("check-rpc-codegen-hygiene: ok (4 checks)");
