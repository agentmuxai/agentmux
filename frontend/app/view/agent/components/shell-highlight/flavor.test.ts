// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { detectShellFlavor } from "./flavor";

describe("detectShellFlavor", () => {
    it.each([
        "pwsh -NoProfile -Command \"Get-ChildItem\"",
        "powershell.exe -File build.ps1",
        "Get-ChildItem -Recurse | Where-Object { $_.Length -gt 1MB }",
        "cd x; Remove-Item -Recurse -Force dist",
        "echo $env:PATH",
        "& pwsh -c ls",
    ])("reads %s as PowerShell", (cmd) => {
        expect(detectShellFlavor(cmd)).toBe("powershell");
    });

    it.each(["cmd /c dir", "echo %USERPROFILE%", "if exist build rmdir /s /q build", "@echo off"])(
        "reads %s as cmd",
        (cmd) => {
            expect(detectShellFlavor(cmd)).toBe("cmd");
        }
    );

    it.each([
        "git status",
        "ls -la | grep foo",
        "date +%Y%m%d",
        "printf '%s%d\\n' a 1",
        'curl -H "Set-Cookie: a=b" https://example.com',
        "echo $PATH",
        "cd x && cargo test",
        "",
    ])("falls back to POSIX for %s", (cmd) => {
        expect(detectShellFlavor(cmd)).toBe("posix");
    });
});
