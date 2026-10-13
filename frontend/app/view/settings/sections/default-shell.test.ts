// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ShellInfo, ShellList } from "@/app/store/rpc-api/shells";
import { automaticLabel, CUSTOM, describeEffective, selectedEntry, shellLabel } from "./default-shell-model";

const pwsh: ShellInfo = { path: String.raw`C:\Program Files\PowerShell\7\pwsh.exe`, name: "PowerShell", kind: "pwsh", version: "7.6.6" };
const gitBash: ShellInfo = { path: String.raw`C:\Program Files\Git\bin\bash.exe`, name: "Git Bash", kind: "gitbash", version: "5.2.37" };
const cmd: ShellInfo = { path: String.raw`C:\Windows\System32\cmd.exe`, name: "Command Prompt", kind: "cmd" };
const shells = [pwsh, gitBash, cmd];

const listWith = (effective: ShellList["effective"]): ShellList => ({ shells, automatic: pwsh.path, effective });

describe("default shell picker", () => {
    it("labels a shell with its version when it gave one", () => {
        expect(shellLabel(pwsh)).toBe("PowerShell 7.6.6");
        expect(shellLabel(cmd)).toBe("Command Prompt");
    });

    it("selects Automatic, a listed shell, or Custom from the setting", () => {
        expect(selectedEntry(undefined, shells)).toBe("");
        expect(selectedEntry("  ", shells)).toBe("");
        // Either slash and any case: the same file on Windows.
        expect(selectedEntry("c:/program files/git/bin/bash.exe", shells)).toBe(gitBash.path);
        expect(selectedEntry(String.raw`D:\tools\nu.exe`, shells)).toBe(CUSTOM);
    });

    it("names what Automatic resolves to", () => {
        expect(automaticLabel(null)).toBe("Automatic");
        expect(automaticLabel(listWith({ path: pwsh.path, source: "automatic" }))).toBe("Automatic: PowerShell 7.6.6");
    });

    it("says what new terminals open with, and warns when the chosen shell is gone", () => {
        expect(describeEffective(listWith({ path: pwsh.path, source: "automatic" }))).toEqual({
            text: String.raw`New terminals open with PowerShell 7.6.6 (C:\Program Files\PowerShell\7\pwsh.exe), chosen automatically.`,
            warn: false,
        });
        expect(describeEffective(listWith({ path: gitBash.path, source: "setting" })).text).toBe(
            String.raw`New terminals open with Git Bash 5.2.37 (C:\Program Files\Git\bin\bash.exe).`
        );
        const fallback = describeEffective(listWith({ path: pwsh.path, source: "fallback", missing: String.raw`D:\gone\nu.exe` }));
        expect(fallback.warn).toBe(true);
        expect(fallback.text).toMatch(/^D:\\gone\\nu\.exe wasn't found, so new terminals open with PowerShell 7\.6\.6/);
    });
});
