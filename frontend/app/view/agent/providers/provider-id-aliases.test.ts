// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Drift guard for the provider-ID alias table duplicated across the frontend
// and the Rust backend — same idiom as pin-consistency.test.ts. Extracts
// agentmux-srv/src/backend/providers.rs's `ALIASES` map by regex and asserts
// every entry matches provider-id-aliases.ts's copy, in both directions (so a
// backend addition that's never mirrored here fails loudly instead of
// silently reintroducing the alias-mismatch bug this file exists to fix —
// see SPEC_AGENT_PANE_MOUNT_AUTH_CHECK_WRONG_DIR_2026_07_31.md).
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { _PROVIDER_ID_ALIASES_FOR_TEST as frontendAliases, canonicalProviderId, lastLinkedAccountId } from "./provider-id-aliases";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../..");

function readSrvAliases(): Record<string, string> {
    const source = readFileSync(resolve(repoRoot, "agentmux-srv/src/backend/providers.rs"), "utf8");
    const match = source.match(/static ALIASES:[\s\S]*?LazyLock::new\(\|\| \{([\s\S]*?)\n\}\);/);
    if (!match) throw new Error("ALIASES map not found in agentmux-srv/src/backend/providers.rs");
    const body = match[1];
    const entries: Record<string, string> = {};
    for (const m of body.matchAll(/m\.insert\("([^"]+)",\s*"([^"]+)"\)/g)) {
        entries[m[1]] = m[2];
    }
    if (Object.keys(entries).length === 0) throw new Error("no m.insert(...) entries parsed from ALIASES map");
    return entries;
}

describe("provider ID alias table — frontend/backend consistency", () => {
    const srvAliases = readSrvAliases();

    it("every backend alias has an identical frontend entry", () => {
        for (const [alias, canonical] of Object.entries(srvAliases)) {
            expect(frontendAliases[alias], `frontend missing/mismatched alias "${alias}"`).toBe(canonical);
        }
    });

    it("every frontend alias has an identical backend entry (no orphaned/stale entries)", () => {
        for (const [alias, canonical] of Object.entries(frontendAliases)) {
            expect(srvAliases[alias], `backend missing/mismatched alias "${alias}"`).toBe(canonical);
        }
    });

    it("canonicalProviderId resolves a known alias and passes through an unknown/canonical ID unchanged", () => {
        expect(canonicalProviderId("claude-code")).toBe("claude");
        expect(canonicalProviderId("claude")).toBe("claude");
        expect(canonicalProviderId("some-unrecognized-id")).toBe("some-unrecognized-id");
    });

    describe("lastLinkedAccountId", () => {
        it("returns undefined when no link matches", () => {
            expect(lastLinkedAccountId([{ provider: "codex", account_id: "acct-1" }], "claude")).toBeUndefined();
            expect(lastLinkedAccountId([], "claude")).toBeUndefined();
        });

        it("matches a canonical link directly", () => {
            expect(lastLinkedAccountId([{ provider: "claude", account_id: "acct-1" }], "claude")).toBe("acct-1");
        });

        it("matches a link stored under a legacy alias", () => {
            expect(lastLinkedAccountId([{ provider: "claude-code", account_id: "acct-1" }], "claude")).toBe("acct-1");
        });

        it("prefers the LAST canonical-equivalent match, mirroring the backend's HashMap::insert overwrite order", () => {
            const links = [
                { provider: "claude", account_id: "acct-canonical" },
                { provider: "claude-code", account_id: "acct-alias" },
            ];
            expect(lastLinkedAccountId(links, "claude")).toBe("acct-alias");
        });
    });
});
