// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import { canShare, isShared } from "./vault-share";
import { promptTitle } from "./vault-use-question";

const entry = (kind: string, fields: Record<string, string>): VaultEntryInfo =>
    ({ id: "login-x", kind, section: "logins", label: "Intranet", vault_id: "personal", fields, tags: [], parts_set: [], grants: [], tail: "", versions: 0, created_at: 1, updated_at: 1, last_used_at: 0 }) as VaultEntryInfo;

describe("sharing a login with an agent", () => {
    it("offers Share only for a login saved for a site, with no realm or proxy", () => {
        expect(canShare(entry("login", { site: "https://a.example.com" }))).toBe(true);
        expect(canShare(entry("login", {}))).toBe(false);
        expect(canShare(entry("login", { site: "https://a.example.com", realm: "Staff" }))).toBe(false);
        expect(canShare(entry("login", { site: "https://a.example.com", proxy: "yes" }))).toBe(false);
        expect(canShare(entry("api-key", { site: "https://a.example.com" }))).toBe(false);
    });

    it("knows a login is shared from its fields", () => {
        expect(isShared(entry("login", { site: "https://a.example.com", shared: "uid-1" }))).toBe(true);
        expect(isShared(entry("login", { site: "https://a.example.com" }))).toBe(false);
    });

    it("titles the person's question with the agent, the site and the login", () => {
        const u = { agent: "Pulp", entry_label: "Intranet", origin: "https://intranet.example.com", share: true } as UseRequest;
        expect(promptTitle(u)).toBe("Pulp wants to sign in to intranet.example.com with Intranet");
    });
});
