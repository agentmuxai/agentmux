#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sign AgentMux widget packages (docs/specs/SPEC_WIDGET_SHARING_2026_10_10.md §2).
 *
 *   agentmux-widget keygen <key-file>          a new Ed25519 key; keep it private
 *   agentmux-widget sign <folder> --key <file> writes <folder>/widget.sig
 *   agentmux-widget verify <folder>            checks widget.sig, prints the fingerprint
 *
 * Only the command line: the signing itself is widget-sign.mjs. This file
 * always runs main, so it works however it's started (a bin link, npx).
 */

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fingerprint, manifestOf, newKey, SIG_FILE, signPackage, verifyPackage } from "./widget-sign.mjs";

function main(argv) {
    const [cmd, target, ...rest] = argv;
    const opt = (name) => {
        const i = rest.indexOf(name);
        return i >= 0 ? rest[i + 1] : undefined;
    };
    if (cmd === "keygen" && target) {
        if (existsSync(target)) throw new Error(`${target} exists; pick another file`);
        const key = newKey();
        writeFileSync(target, JSON.stringify({ algorithm: "ed25519", ...key }, null, 2) + "\n", { mode: 0o600 });
        console.log(`Wrote ${target}. Keep it private. Your key's fingerprint: ${fingerprint(key.publicKey)}`);
        return;
    }
    if (cmd === "sign" && target && opt("--key")) {
        const dir = resolve(target);
        const key = JSON.parse(readFileSync(opt("--key"), "utf8"));
        writeFileSync(join(dir, SIG_FILE), signPackage(dir, key.seed));
        const m = manifestOf(dir);
        console.log(`Signed ${m.id} ${m.version} · ${fingerprint(key.publicKey)}`);
        return;
    }
    if (cmd === "verify" && target) {
        const dir = resolve(target);
        console.log(`${manifestOf(dir).id}: signed by ${verifyPackage(dir)}`);
        return;
    }
    console.error("usage: agentmux-widget keygen <key-file> | sign <folder> --key <key-file> | verify <folder>");
    process.exitCode = 2;
}

try {
    main(process.argv.slice(2));
} catch (e) {
    console.error(e instanceof Error ? e.message : String(e));
    process.exitCode = 1;
}
