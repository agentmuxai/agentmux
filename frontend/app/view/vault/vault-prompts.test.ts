// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { firstSaveNotice, vaultPromptNotes } from "./vault-prompts";

describe("what the Vault may ask the person", () => {
    it("names the key store and says nothing needs administrator rights, on every platform", () => {
        for (const [platform, store] of [
            ["win32", "Windows Credential Manager"],
            ["darwin", "the macOS Keychain"],
            ["linux", "your desktop's keyring"],
        ]) {
            const notes = vaultPromptNotes(platform).join(" ");
            expect(notes).toContain(store);
            expect(notes).toContain("never asks for administrator rights");
            expect(notes).toContain("Agents never see these prompts");
        }
    });

    it("says what each platform asks, and when", () => {
        expect(vaultPromptNotes("win32").join(" ")).toContain("doesn't ask you anything");
        expect(vaultPromptNotes("win32").join(" ")).toContain("Windows Hello");
        expect(vaultPromptNotes("darwin").join(" ")).toContain("need Windows Hello for now");
        expect(vaultPromptNotes("darwin").join(" ")).toMatch(/first time.*Always Allow/);
        expect(vaultPromptNotes("linux").join(" ")).toContain("keyring is locked");
    });

    it("warns before the first save only where something will ask", () => {
        expect(firstSaveNotice("win32")).toBeNull();
        expect(firstSaveNotice("darwin")).toContain("Always Allow");
        expect(firstSaveNotice("linux")).toContain("keyring");
    });
});
