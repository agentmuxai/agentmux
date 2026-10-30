// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** An entry's expiry date as the pane reads it (spec §15.2 of
 *  SPEC_VAULT_GENERATE_CAPTURE_AND_BROWSER_SIGNINS_2026_10_28.md). */

import { describe, expect, it } from "vitest";

import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import { entryDetails, entryMatches, expiringSoon } from "./vault-entries";
import { expiryOf } from "./vault-format";

const now = new Date(2026, 10, 10, 15, 30); // 2026-11-10, mid-afternoon

const entry = (fields: Record<string, string>): VaultEntryInfo => ({
    id: "api-key-stripe",
    kind: "api-key",
    section: "api-keys",
    label: "Stripe",
    fields,
    tags: [],
    parts_set: ["value"],
    grants: [],
    tail: "ab12",
    versions: 0,
    created_at: 0,
    updated_at: 0,
    last_used_at: 0,
});

describe("expiryOf", () => {
    it("counts whole days from today, in local time", () => {
        expect(expiryOf("2026-11-30", now)).toEqual({ days: 20, text: "expires in 20 days", warn: false });
        expect(expiryOf("2026-11-24", now)?.warn).toBe(true);
        expect(expiryOf("2026-11-11", now)?.text).toBe("expires tomorrow");
        expect(expiryOf("2026-11-10", now)).toEqual({ days: 0, text: "expires today", warn: true });
        expect(expiryOf("2026-11-01", now)).toEqual({ days: -9, text: "expired on 2026-11-01", warn: true });
    });

    it("reads nothing that isn't a real date", () => {
        for (const bad of [undefined, "", "Dec 2026", "2026-02-30", "2026-13-01"]) {
            expect(expiryOf(bad, now)).toBeNull();
        }
    });
});

describe("the entry list", () => {
    it("shows the date on its own line, and still finds an entry by it", () => {
        const e = entry({ service: "Stripe", expires: "2026-11-30" });
        expect(entryDetails(e)).toBe("Stripe");
        expect(entryMatches(e, "2026-11")).toBe(true);
    });

    it("counts entries past their date or within 30 days as expiring soon", () => {
        expect(expiringSoon(entry({ expires: "2026-12-10" }), now)).toBe(true);
        expect(expiringSoon(entry({ expires: "2026-10-01" }), now)).toBe(true);
        expect(expiringSoon(entry({ expires: "2027-01-31" }), now)).toBe(false);
        expect(expiringSoon(entry({}), now)).toBe(false);
    });
});
