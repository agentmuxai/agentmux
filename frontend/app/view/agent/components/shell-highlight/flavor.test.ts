// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { detectShellFlavor } from "./flavor";
import { MAX_TOKENIZE_CHARS } from "./tokenize";

describe("detectShellFlavor", () => {
    it.each([
        "pwsh -NoProfile -Command \"Get-ChildItem\"",
        "powershell.exe -File build.ps1",
        "Get-ChildItem -Recurse | Where-Object { $_.Length -gt 1MB }",
        "cd x; Remove-Item -Recurse -Force dist",
        "echo $env:PATH",
        "& pwsh -c ls",
        `"C:\\Program Files\\PowerShell\\7\\pwsh.exe" -Command 'Write-Output x'`,
        "C:/Windows/System32/WindowsPowerShell/v1.0/powershell.exe -NoProfile -c x",
        "/usr/bin/pwsh -c x",
    ])("reads %s as PowerShell", (cmd) => {
        expect(detectShellFlavor(cmd)).toBe("powershell");
    });

    it.each([
        "cmd /c dir",
        "echo %USERPROFILE%",
        'copy a.txt %TEMP%\\a.txt',
        "if exist build rmdir /s /q build",
        "@echo off",
    ])(
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
        "pwshell-tool --help",
        "git log --pretty=format:%h%x09%s",
        "git log --format=%h%x09%an%x09%s -5",
        `python -c "print('100%abc%')"`,
        "printf '%FOO%'",
        'echo "literal %PATH% text"',
        "",
    ])("falls back to POSIX for %s", (cmd) => {
        expect(detectShellFlavor(cmd)).toBe("posix");
    });

    it.each([
        "cat > r.txt <<EOF\nSet-Cookie: a=b\nEOF",
        'git commit -m "fix\nAdd-On support"',
        "echo 'x\nWrite-Up'",
        "ls # Get-ChildItem",
    ])("does not read cmdlet-looking text inside strings, heredocs or comments as PowerShell: %s", (cmd) => {
        expect(detectShellFlavor(cmd)).toBe("posix");
    });

    it("does not judge text past the tokenizer cap, where nothing is masked", () => {
        const pad = "x".repeat(MAX_TOKENIZE_CHARS);
        expect(detectShellFlavor(`cat > a.txt <<'EOF'\n${pad}\nWrite-Host hi\nEOF`)).toBe("posix");
        expect(detectShellFlavor(`python -c "${pad} %PATH% "`)).toBe("posix");
    });

    it("still reads a multi-line PowerShell script as PowerShell", () => {
        expect(detectShellFlavor('$x = 1\nGet-ChildItem "C:\\a"\nWrite-Host $x')).toBe("powershell");
    });
});
