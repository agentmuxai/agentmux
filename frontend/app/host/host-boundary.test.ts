// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The host boundary (docs/specs/SPEC_HOST_API_SEAM_2026_09_26.md): only the
 * CEF host implementation may talk to the CEF host directly — by importing
 * `app/platform/ipc` (`invokeCommand`, `listenEvent`, `invokeBrowserApi`) or
 * reading the `__AGENTMUX_IPC_*` globals. Everything else goes through
 * `getApi()`, so the UI runs on any host that implements `AppApi`.
 *
 * The seam is the CEF host implementation plus the CEF entry: code that runs
 * before `window.api` exists (bootstrap, its log transport) or precisely when
 * it has failed (the boot-error recovery). A different host has its own entry.
 *
 * PENDING held the files still bypassing the seam while slices 1–5 moved them
 * (it is empty now). It stays as the ratchet's shape: a new file bypassing the
 * seam fails this test.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { describe, expect, it } from "vitest";

const FRONTEND_ROOT = join(__dirname, "..", "..");

/** Allowed to reach the CEF host directly. */
const SEAM = [
    // The CEF host implementation of AppApi.
    "app/host/cef-host-commands.ts",
    "app/init/host-detect.ts",
    "app/platform/ipc.ts",
    "cef-init.ts",
    "types/custom.d.ts",
    "util/cef-api.ts",
    // The CEF entry. bootstrap.ts is index.html's module; it installs the log
    // pipe and error forwarder before window.api exists, then sets it up.
    "bootstrap.ts",
    "log/error-forwarder.ts",
    "log/log-pipe.ts",
    // Boot-error recovery: runs when window.api failed, so it cannot use it.
    // Without CEF's IPC credentials it does nothing.
    "app/init/error-display.ts",
];

/** Files still bypassing the seam. Empty: add nothing here. */
const PENDING: string[] = [];

// Static `from "…"` and dynamic `import("…")` alike.
const IPC_MODULE = String.raw`["'](?:@\/app\/platform\/ipc|(?:\.\.?\/)+(?:app\/)?platform\/ipc|\.\/ipc)["']`;
const IMPORTS_IPC = new RegExp(String.raw`(?:from\s+|import\(\s*)` + IPC_MODULE);
const READS_IPC_GLOBALS = /__AGENTMUX_IPC_/;

function collectSourceFiles(dir: string, out: string[] = []): string[] {
    for (const entry of readdirSync(dir)) {
        const full = join(dir, entry);
        if (statSync(full).isDirectory()) {
            if (entry === "node_modules" || entry === "test") continue;
            collectSourceFiles(full, out);
        } else if (/\.(ts|tsx)$/.test(entry) && !/\.test\.tsx?$/.test(entry)) {
            out.push(full);
        }
    }
    return out;
}

function filesReachingTheHost(): string[] {
    return collectSourceFiles(FRONTEND_ROOT)
        .filter((file) => {
            const text = readFileSync(file, "utf8");
            return IMPORTS_IPC.test(text) || READS_IPC_GLOBALS.test(text);
        })
        .map((file) => relative(FRONTEND_ROOT, file).split(sep).join("/"))
        .sort();
}

describe("host boundary", () => {
    it("only the seam and the pending list reach the CEF host directly", () => {
        const bypassing = filesReachingTheHost().filter((f) => !SEAM.includes(f));
        expect(
            bypassing,
            "A file reaches the CEF host directly. Add a method to AppApi (types/custom.d.ts, util/cef-api.ts) " +
                "and call it through getApi() instead. If you just removed the last direct call from a file, " +
                "delete it from PENDING in this test."
        ).toEqual([...PENDING].sort());
    });

    it("every seam file still exists and still reaches the host", () => {
        const reaching = filesReachingTheHost();
        for (const f of SEAM) {
            expect(reaching, `${f} is listed as seam but no longer reaches the host`).toContain(f);
        }
    });
});
