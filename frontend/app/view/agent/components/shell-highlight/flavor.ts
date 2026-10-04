// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which shell a Bash-tool command is written for. The tool runs PowerShell and
 * cmd as well as POSIX shells (agents on Windows), and the POSIX tokenizer
 * mis-colours both. Ambiguity falls back to POSIX.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_BASH_HIGHLIGHTING_2026_10_04.md §3.1
 */

import { MAX_TOKENIZE_CHARS, tokenizeShell, type TokenKind } from "./tokenize";

export type ShellFlavor = "posix" | "powershell" | "cmd";

// `pwsh`, `powershell.exe`, or either by path, quoted or not
// (`"C:\Program Files\PowerShell\7\pwsh.exe" -Command …`).
const POWERSHELL_LAUNCH_RE =
    /^\s*(?:&\s*)?(?:"(?:[^"\n]*[\\/])?|'(?:[^'\n]*[\\/])?|(?:[^\s"']*[\\/])?)(?:powershell|pwsh)(?:\.exe)?["']?(?=\s|$)/i;
// A cmdlet (`Verb-Noun`, capitalised) where a command can start: not a header
// value inside a string such as `-H "Set-Cookie: x"`.
const CMDLET_RE =
    /(?:^|[|;({&]|\n)\s*(?:Get|Set|New|Remove|Select|Where|ForEach|Invoke|Write|Out|Test|Start|Stop|Add|Import|Export|Copy|Move|Join|Split|Resolve|Format|Measure|Sort|Compare|Clear|Rename|Restart|Enable|Disable|Install|Uninstall)-[A-Z][A-Za-z]+/;
const POWERSHELL_VAR_RE = /\$env:[A-Za-z_]|\$\([ ]*Get-[A-Z]|\$_\.[A-Za-z]/;
const CMD_LAUNCH_RE = /^\s*cmd(?:\.exe)?\s+\/[ck]\b/i;
// A whole `%NAME%` word part (3+ characters, bounded by a separator), so
// `date +%Y%m%d`, git's `--format=%h%x09%s` and printf formats are not cmd.
const CMD_PERCENT_VAR_RE = /(?:^|[\s"'=;(\\])%[A-Za-z_][A-Za-z0-9_]{2,}%(?=$|[\s"'\\;)/:.])/;
const CMD_BUILTIN_RE = /^\s*(?:@?echo\s+off|if\s+(?:not\s+)?exist\b|cd\s+\/d\b)/i;

const MASKED: ReadonlySet<TokenKind> = new Set(["string", "heredoc-body", "heredoc-marker", "comment"]);

/**
 * `command` with quoted text, heredoc bodies and comments blanked (same
 * length), so a line inside them that happens to start `Set-Cookie` or
 * `Add-On` is not read as a cmdlet.
 */
function maskQuoted(command: string): string {
    let out = "";
    let pos = 0;
    for (const t of tokenizeShell(command).tokens) {
        if (!MASKED.has(t.kind)) continue;
        out += command.slice(pos, t.start) + command.slice(t.start, t.end).replace(/[^\n]/g, " ");
        pos = t.end;
    }
    return out + command.slice(pos);
}

export function detectShellFlavor(full: string): ShellFlavor {
    // Judge only what the tokenizer covers: past its cap nothing is masked, so
    // a long heredoc's tail could otherwise pass for PowerShell or cmd code.
    const command = full.length > MAX_TOKENIZE_CHARS ? full.slice(0, MAX_TOKENIZE_CHARS) : full;
    if (POWERSHELL_LAUNCH_RE.test(command)) return "powershell";
    if (CMDLET_RE.test(command) || POWERSHELL_VAR_RE.test(command)) {
        // A cheap hit; confirm it is not text inside a string or heredoc.
        const code = maskQuoted(command);
        if (CMDLET_RE.test(code) || POWERSHELL_VAR_RE.test(code)) return "powershell";
    }
    if (CMD_LAUNCH_RE.test(command) || CMD_BUILTIN_RE.test(command)) return "cmd";
    // Only unquoted `%NAME%`: inside quotes it is as likely a printf format or
    // literal text in a POSIX command, and POSIX is the safe fallback.
    if (CMD_PERCENT_VAR_RE.test(command) && CMD_PERCENT_VAR_RE.test(maskQuoted(command))) return "cmd";
    return "posix";
}
