// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import { entryIdOf, keyChoices, keyState, referenceTo } from "./vault-key-field-model";

const entry = (id: string, kind: string, label: string) => ({ id, kind, label }) as VaultEntryInfo;
const entries = [entry("b", "api-key", "Work key"), entry("a", "api-key", "Another key"), entry("n", "note", "A note")];

describe("a settings key kept in the Vault", () => {
    it("names its entry as vault:<id>, the spelling srv reads", () => {
        expect(referenceTo("abc")).toBe("vault:abc");
        expect(entryIdOf("vault:abc")).toBe("abc");
        expect(entryIdOf("  vault: abc ")).toBe("abc");
        expect(entryIdOf("vault:")).toBeUndefined();
        expect(entryIdOf("••••••••")).toBeUndefined();
        expect(entryIdOf(undefined)).toBeUndefined();
    });

    it("offers the API keys by label and no other entry", () => {
        expect(keyChoices(entries).map((e) => e.label)).toEqual(["Another key", "Work key"]);
    });

    it("tells unset, plain, in place and gone apart, and does not call a reference gone before the Vault loads", () => {
        expect(keyState(undefined, entries)).toBe("unset");
        expect(keyState("  ", entries)).toBe("unset");
        expect(keyState("••••••••", entries)).toBe("plain");
        expect(keyState("vault:a", entries)).toBe("vault");
        expect(keyState("vault:n", entries)).toBe("gone");
        expect(keyState("vault:zzz", entries)).toBe("gone");
        expect(keyState("vault:zzz", null)).toBe("vault");
    });
});
