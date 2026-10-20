// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the person may be asked while using the Vault, said before it happens
// (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.7).
// Each later step that adds a prompt (Reveal and Copy, encrypted imports)
// adds its line here.

export type VaultPlatform = "win32" | "darwin" | "linux" | string;

/** Where the Vault key is kept, in the platform's own words. */
export function keyStoreName(platform: VaultPlatform): string {
    if (platform === "win32") return "Windows Credential Manager";
    if (platform === "darwin") return "the macOS Keychain";
    return "your desktop's keyring";
}

/** Every prompt the Vault can cause on this platform, in the order they come. */
export function vaultPromptNotes(platform: VaultPlatform): string[] {
    const notes = [`Your entries are encrypted with a Vault key kept in ${keyStoreName(platform)}, under your own account.`];
    if (platform === "win32") {
        notes.push("Windows doesn't ask you anything to save, change or list entries.");
    } else if (platform === "darwin") {
        notes.push(
            "The first time AgentMux uses the Vault key, and again after AgentMux updates, macOS asks whether AgentMux may use its keychain item. Choose Always Allow; Deny leaves the Vault unable to open its entries."
        );
    } else {
        notes.push("If your keyring is locked, your desktop asks for its password the first time AgentMux uses the Vault key.");
    }
    if (platform === "win32") {
        notes.push("Reveal and Copy ask Windows Hello (your PIN, fingerprint or face) each time, to confirm it's you. A copied value clears from the clipboard after 30 seconds.");
    } else {
        notes.push("Reveal and Copy need Windows Hello for now; Touch ID and the desktop's prompt aren't built yet.");
    }
    notes.push("The Vault never asks for administrator rights. Agents never see these prompts, and can't answer them.");
    return notes;
}

/** Said just before the first save, which creates the Vault key; null when nothing will ask. */
export function firstSaveNotice(platform: VaultPlatform): string | null {
    if (platform === "darwin") return "Saving creates your Vault key: macOS asks whether AgentMux may use the keychain. Choose Always Allow.";
    if (platform === "linux") return "Saving creates your Vault key: if your keyring is locked, your desktop asks for its password.";
    return null;
}
