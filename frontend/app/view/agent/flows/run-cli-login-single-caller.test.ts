// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pins the topology fix from retro-login-three-code-paths-2026-07-20: the
 * raw host primitive `getApi().runCliLogin` must have exactly one caller in
 * the whole frontend — `force-login.ts`, which `runProviderLogin` wraps.
 *
 * The previous incident wasn't a logic bug in the fallback tiers; it was a
 * SECOND, independent call site (`launch-flow.ts`) that spawned the login
 * CLI directly and never got the "no URL captured" fallback a sibling call
 * site already had. Grepping for callers of a well-named helper
 * (`forceProviderLogin`) missed it precisely because it didn't use that
 * helper. This test grep-shapes the invariant itself — "only force-login.ts
 * may call the raw primitive" — so a future direct call regresses a test,
 * not just a user's "Retry Login" click.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const APP_ROOT = join(__dirname, "..", "..", "..");
const SANCTIONED_CALLER = join(__dirname, "force-login.ts");

function collectSourceFiles(dir: string, out: string[] = []): string[] {
    for (const entry of readdirSync(dir)) {
        const full = join(dir, entry);
        const stat = statSync(full);
        if (stat.isDirectory()) {
            collectSourceFiles(full, out);
        } else if (/\.(ts|tsx)$/.test(entry) && !entry.endsWith(".test.ts") && !entry.endsWith(".test.tsx")) {
            out.push(full);
        }
    }
    return out;
}

describe("provider-login call-site topology", () => {
    // The raw host primitive sits behind the login backend (login-backend.ts,
    // SPEC_PROVIDER_LOGIN_THROUGH_SRV_2026_10_09.md L4), and starting a login
    // through the backend is still force-login.ts's alone.
    const callersOf = (needle: string): string[] =>
        collectSourceFiles(APP_ROOT)
            .filter((file) => readFileSync(file, "utf8").includes(needle))
            .map((file) => relative(APP_ROOT, file));

    it("the raw getApi().runCliLogin is called from exactly one file: login-backend.ts", () => {
        expect(callersOf(".runCliLogin(")).toEqual([relative(APP_ROOT, join(__dirname, "login-backend.ts"))]);
    });

    it("a login is started from exactly one file: force-login.ts", () => {
        expect(callersOf("loginBackend().start(")).toEqual([relative(APP_ROOT, SANCTIONED_CALLER)]);
    });
});
