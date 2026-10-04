// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which shell a Bash-tool command is written for. The tool runs PowerShell and
 * cmd as well as POSIX shells (agents on Windows), and the POSIX tokenizer
 * mis-colours both. Ambiguity falls back to POSIX.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_BASH_HIGHLIGHTING_2026_10_04.md §3.1
 */

export type ShellFlavor = "posix" | "powershell" | "cmd";

const POWERSHELL_LAUNCH_RE = /^\s*(?:&\s*)?(?:powershell|pwsh)(?:\.exe)?\b/i;
// A cmdlet (`Verb-Noun`, capitalised) where a command can start: not a header
// value inside a string such as `-H "Set-Cookie: x"`.
const CMDLET_RE =
    /(?:^|[|;({&]|\n)\s*(?:Get|Set|New|Remove|Select|Where|ForEach|Invoke|Write|Out|Test|Start|Stop|Add|Import|Export|Copy|Move|Join|Split|Resolve|Format|Measure|Sort|Compare|Clear|Rename|Restart|Enable|Disable|Install|Uninstall)-[A-Z][A-Za-z]+/;
const POWERSHELL_VAR_RE = /\$env:[A-Za-z_]|\$\([ ]*Get-[A-Z]|\$_\.[A-Za-z]/;
const CMD_LAUNCH_RE = /^\s*cmd(?:\.exe)?\s+\/[ck]\b/i;
// 3+ characters, so `date +%Y%m%d` and printf formats are not read as cmd.
const CMD_PERCENT_VAR_RE = /%[A-Za-z_][A-Za-z0-9_]{2,}%/;
const CMD_BUILTIN_RE = /^\s*(?:@?echo\s+off|if\s+(?:not\s+)?exist\b|cd\s+\/d\b)/i;

export function detectShellFlavor(command: string): ShellFlavor {
    if (POWERSHELL_LAUNCH_RE.test(command) || CMDLET_RE.test(command) || POWERSHELL_VAR_RE.test(command)) {
        return "powershell";
    }
    if (CMD_LAUNCH_RE.test(command) || CMD_BUILTIN_RE.test(command) || CMD_PERCENT_VAR_RE.test(command)) {
        return "cmd";
    }
    return "posix";
}
