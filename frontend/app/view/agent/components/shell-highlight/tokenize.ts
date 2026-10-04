// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tolerant tokenizer for POSIX shell commands, for presentation only.
 *
 * Returns ranges into the original string, never copies of it, and the ranges
 * always tile the input: concatenating every token's text gives back the exact
 * command. Anything the scanner is unsure about is `plain`. It never throws.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_BASH_HIGHLIGHTING_2026_10_04.md §3.1
 */

export type TokenKind =
    | "plain"
    | "program"
    | "subcommand"
    | "keyword"
    | "flag"
    | "flag-value"
    | "string"
    | "variable"
    | "substitution"
    | "operator"
    | "redirect"
    | "path"
    | "url"
    | "number"
    | "comment"
    | "heredoc-marker"
    | "heredoc-body"
    | "env-assign"
    | "punct";

export interface Token {
    start: number;
    end: number;
    kind: TokenKind;
}

/** One simple command at the top level: the text between `| && ; newline`. */
export interface Segment {
    start: number;
    end: number;
    /** The program word, or null for a segment with none (a bare assignment). */
    program: { start: number; end: number } | null;
}

export interface Tokenized {
    tokens: Token[];
    segments: Segment[];
}

/** Longer commands are tokenized up to here; the rest is one `plain` token. */
export const MAX_TOKENIZE_CHARS = 20_000;

/** Words after which the next word is a command again. */
const KEYWORDS_THEN_COMMAND = new Set(["if", "then", "else", "elif", "while", "until", "do", "{", "!", "time"]);
/** Keywords after which the next word is an ordinary argument. */
const KEYWORDS_THEN_ARGS = new Set(["for", "case", "select", "function", "in", "fi", "done", "esac", "}"]);

/** Programs whose next non-flag word is itself a program. */
const WRAPPERS = new Set(["sudo", "doas", "env", "nohup", "exec", "command", "nice", "builtin", "time"]);

/** Wrapper options whose value is the next word (`sudo -u root rm`), so that word is not the program. */
const WRAPPER_VALUE_OPTIONS: Record<string, ReadonlySet<string>> = {
    sudo: new Set(["-u", "-g", "-h", "-p", "-C", "-D", "-r", "-t", "-U", "-T", "--user", "--group", "--host", "--prompt", "--close-from", "--chdir", "--role", "--type", "--other-user", "--command-timeout"]),
    doas: new Set(["-u", "-C"]),
    env: new Set(["-u", "-C", "-S", "--unset", "--chdir", "--split-string"]),
    nice: new Set(["-n", "--adjustment"]),
    time: new Set(["-f", "-o", "--format", "--output"]),
    exec: new Set(["-a"]),
};

/** Programs with subcommands, and how many consecutive subcommand words they take. */
const SUBCOMMAND_DEPTH: Record<string, number> = {
    git: 1,
    cargo: 1,
    npm: 1,
    pnpm: 1,
    yarn: 1,
    npx: 0,
    docker: 2,
    podman: 2,
    gh: 2,
    "gh-agent": 2,
    kubectl: 1,
    go: 1,
    pip: 1,
    pip3: 1,
    uv: 1,
    rustup: 1,
    systemctl: 1,
    brew: 1,
    apt: 1,
    "apt-get": 1,
    dotnet: 1,
    aws: 2,
    task: 0,
};

/** Deepest `$(` / backtick nesting scanned; deeper is left as plain text. */
const MAX_NEST = 32;

const URL_RE = /^[a-z][a-z0-9+.-]*:\/\//i;
const PATH_RE = /^(?:~|[A-Za-z]:[\\/])|[\\/]/;
const NUMBER_RE = /^-?\d+(?:\.\d+)?$/;
const ENV_ASSIGN_RE = /^[A-Za-z_][A-Za-z0-9_]*\+?=/;
const SUBCOMMAND_RE = /^[a-z][a-z0-9:_-]*$/;
const VARIABLE_RE = /\$(?:[A-Za-z_][A-Za-z0-9_]*|[0-9@?$!#*-])/y;
const REDIRECT_RE = /(?:[0-9]+|&)?(?:>>|>\||>&[0-9]*-?|>|<>|<&[0-9]*-?|<)/y;
const HEREDOC_RE = /<<(-?)[ \t]*(?:'([^'\n]*)'|"([^"\n]*)"|\\?([A-Za-z0-9_.-]+))/y;

function classifyArg(run: string): TokenKind {
    if (URL_RE.test(run)) return "url";
    if (PATH_RE.test(run)) return "path";
    if (NUMBER_RE.test(run)) return "number";
    return "plain";
}

function isWordBreak(c: string): boolean {
    return (
        c === " " ||
        c === "\t" ||
        c === "\n" ||
        c === "\r" ||
        c === "|" ||
        c === "&" ||
        c === ";" ||
        c === "<" ||
        c === ">" ||
        c === "(" ||
        c === ")"
    );
}

/** Basename of a program path, lowercased, without a Windows executable suffix. */
function programName(run: string): string {
    const base = run.slice(Math.max(run.lastIndexOf("/"), run.lastIndexOf("\\")) + 1).toLowerCase();
    return base.replace(/\.(exe|cmd|bat)$/, "");
}

class Scanner {
    i = 0;
    readonly out: Token[] = [];
    readonly segments: Segment[] = [];
    private pending: { delim: string; strip: boolean }[] = [];
    private segStart = -1;
    private segProgram: Segment["program"] = null;
    private nest = 0;

    constructor(private readonly s: string) {}

    private emit(start: number, end: number, kind: TokenKind): void {
        if (end > start) this.out.push({ start, end, kind });
    }

    private openSegment(at: number): void {
        if (this.segStart < 0) this.segStart = at;
    }

    private closeSegment(end: number): void {
        if (this.segStart >= 0 && end > this.segStart) {
            this.segments.push({ start: this.segStart, end, program: this.segProgram });
        }
        this.segStart = -1;
        this.segProgram = null;
    }

    /** Heredoc bodies start on the line after the marker; `i` is already there. */
    private consumeHeredocs(): void {
        const s = this.s;
        for (const { delim, strip } of this.pending) {
            const bodyStart = this.i;
            let lineStart = this.i;
            let found = false;
            while (lineStart < s.length) {
                let lineEnd = s.indexOf("\n", lineStart);
                if (lineEnd < 0) lineEnd = s.length;
                let line = s.slice(lineStart, lineEnd);
                if (line.endsWith("\r")) line = line.slice(0, -1);
                if ((strip ? line.replace(/^\t+/, "") : line) === delim) {
                    this.emit(bodyStart, lineStart, "heredoc-body");
                    this.emit(lineStart, lineEnd, "heredoc-marker");
                    this.i = Math.min(lineEnd + 1, s.length);
                    found = true;
                    break;
                }
                lineStart = lineEnd + 1;
            }
            if (!found) {
                this.emit(bodyStart, s.length, "heredoc-body");
                this.i = s.length;
            }
        }
        this.pending = [];
    }

    /**
     * Scan a command list. `stop` is the closer of the enclosing construct:
     * `)` for `$(...)`, a backtick for `` `...` ``, null at the top level. The
     * closer itself is left for the caller to consume.
     */
    scanList(stop: ")" | "`" | null): void {
        this.nest++;
        try {
            this.scanListInner(stop);
        } finally {
            this.nest--;
        }
    }

    private scanListInner(stop: ")" | "`" | null): void {
        const s = this.s;
        const n = s.length;
        const top = stop === null;
        let depth = 0;
        let cmdPos = true;
        let wrapper = false;
        // The previous word was a wrapper option that takes a value.
        let wrapperValuePending = false;
        let program = "";
        let wordIdx = 0;
        let subIdx = 0;

        const open = (at: number): void => {
            if (top && depth === 0) this.openSegment(at);
        };
        const resetCommand = (): void => {
            cmdPos = true;
            wrapper = false;
            wrapperValuePending = false;
            program = "";
            wordIdx = 0;
            subIdx = 0;
        };

        while (this.i < n) {
            const c = s[this.i];
            if (c === " " || c === "\t" || c === "\r") {
                this.i++;
                continue;
            }
            if (c === "\n") {
                if (top && depth === 0) this.closeSegment(this.i);
                this.i++;
                this.consumeHeredocs();
                resetCommand();
                continue;
            }
            if (c === "\\" && s[this.i + 1] === "\n") {
                this.i += 2;
                continue;
            }
            if (c === "#") {
                let end = s.indexOf("\n", this.i);
                if (end < 0) end = n;
                this.emit(this.i, end, "comment");
                this.i = end;
                continue;
            }
            if (stop === "`" && c === "`") return;
            if (c === ")") {
                if (stop === ")" && depth === 0) return;
                this.emit(this.i, this.i + 1, "punct");
                this.i++;
                depth = Math.max(0, depth - 1);
                cmdPos = false;
                continue;
            }
            if (c === "(" && s[this.i + 1] === "(" && cmdPos && this.nest < MAX_NEST) {
                open(this.i);
                this.scanArithmetic(2);
                cmdPos = false;
                continue;
            }
            if (c === "(") {
                open(this.i);
                this.emit(this.i, this.i + 1, "punct");
                this.i++;
                depth++;
                resetCommand();
                continue;
            }

            if (c === "<" && s.startsWith("<<<", this.i)) {
                open(this.i);
                this.emit(this.i, this.i + 3, "redirect");
                this.i += 3;
                continue;
            }
            // Heredoc before generic redirects: `<<` would match as `<`.
            if (c === "<" && s[this.i + 1] === "<" && s[this.i + 2] !== "<") {
                HEREDOC_RE.lastIndex = this.i;
                const m = HEREDOC_RE.exec(s);
                if (m) {
                    open(this.i);
                    this.emit(this.i, this.i + m[0].length, "heredoc-marker");
                    this.pending.push({ delim: m[2] ?? m[3] ?? m[4] ?? "", strip: m[1] === "-" });
                    this.i += m[0].length;
                    continue;
                }
            }
            // Process substitution: `<(cmd)` and `>(cmd)`.
            if ((c === "<" || c === ">") && s[this.i + 1] === "(" && this.nest < MAX_NEST) {
                open(this.i);
                this.emit(this.i, this.i + 2, "substitution");
                this.i += 2;
                this.scanList(")");
                if (s[this.i] === ")") {
                    this.emit(this.i, this.i + 1, "substitution");
                    this.i++;
                }
                continue;
            }
            if (c === "<" || c === ">" || c === "&" || (c >= "0" && c <= "9")) {
                REDIRECT_RE.lastIndex = this.i;
                const m = REDIRECT_RE.exec(s);
                if (m) {
                    open(this.i);
                    this.emit(this.i, this.i + m[0].length, "redirect");
                    this.i += m[0].length;
                    continue;
                }
            }
            if (c === "|" || c === "&" || c === ";") {
                let len = 1;
                const two = s.slice(this.i, this.i + 2);
                if (two === "&&" || two === "||" || two === "|&" || two === ";;") len = 2;
                this.emit(this.i, this.i + len, "operator");
                if (top && depth === 0) this.closeSegment(this.i);
                this.i += len;
                resetCommand();
                continue;
            }

            // A word.
            open(this.i);
            const wordStart = this.i;
            let role: "program" | "arg" | "flag" | "env-value" | "keyword" | "wrapper-value" = cmdPos ? "program" : "arg";
            if (role === "program" && wrapperValuePending) role = "wrapper-value";
            wrapperValuePending = false;
            let firstRun = true;
            let sawEquals = false;
            let programEmitted = false;
            let keywordThenCommand = false;

            while (this.i < n) {
                const ch = s[this.i];
                if (isWordBreak(ch)) break;
                if (stop === "`" && ch === "`") break;
                if (ch === "'") {
                    this.scanSingle();
                    firstRun = false;
                    continue;
                }
                if (ch === '"') {
                    this.scanDouble();
                    firstRun = false;
                    continue;
                }
                if (ch === "`") {
                    this.scanBacktick();
                    firstRun = false;
                    continue;
                }
                if (ch === "$" && this.tryDollar()) {
                    firstRun = false;
                    continue;
                }

                // A run of literal characters.
                const runStart = this.i;
                while (this.i < n) {
                    const rc = s[this.i];
                    if (rc === "\\") {
                        this.i += this.i + 1 < n ? 2 : 1;
                        continue;
                    }
                    if (isWordBreak(rc) || rc === "'" || rc === '"' || rc === "`" || rc === "$") break;
                    this.i++;
                }
                if (this.i === runStart) {
                    // A lone `$` (not a variable): literal.
                    this.i++;
                }
                const run = s.slice(runStart, this.i);
                const atWordStart = firstRun && runStart === wordStart;
                firstRun = false;

                if (atWordStart && role === "program") {
                    if (ENV_ASSIGN_RE.test(run)) {
                        const eq = run.indexOf("=") + 1;
                        this.emit(runStart, runStart + eq, "env-assign");
                        role = "env-value";
                        continue;
                    }
                    const atEnd = this.i >= n || isWordBreak(s[this.i]);
                    if (atEnd && (KEYWORDS_THEN_COMMAND.has(run) || KEYWORDS_THEN_ARGS.has(run))) {
                        this.emit(runStart, this.i, "keyword");
                        keywordThenCommand = KEYWORDS_THEN_COMMAND.has(run);
                        role = "keyword";
                        continue;
                    }
                    if (wrapper && run.startsWith("-")) {
                        this.emit(runStart, this.i, "flag");
                        role = "flag";
                        const atWordEnd = this.i >= n || isWordBreak(s[this.i]);
                        wrapperValuePending = atWordEnd && (WRAPPER_VALUE_OPTIONS[program]?.has(run) ?? false);
                        continue;
                    }
                    this.emit(runStart, this.i, "program");
                    if (top && depth === 0 && this.segProgram === null) {
                        this.segProgram = { start: runStart, end: this.i };
                    }
                    programEmitted = true;
                    program = programName(run);
                    wrapper = WRAPPERS.has(program);
                    wordIdx = 1;
                    subIdx = 0;
                    continue;
                }
                if (role === "program") {
                    // Later runs of a program word (`"$HOME"/bin/x`).
                    this.emit(runStart, this.i, "program");
                    continue;
                }
                if (role === "keyword" || role === "env-value") {
                    continue;
                }
                if (role === "wrapper-value") {
                    const kind = classifyArg(run);
                    if (kind !== "plain") this.emit(runStart, this.i, kind);
                    continue;
                }
                if (atWordStart && role === "arg" && run.length > 1 && run.startsWith("-")) {
                    role = "flag";
                }
                if (role === "flag") {
                    const eq = !sawEquals ? run.indexOf("=") : -1;
                    if (eq > 0) {
                        sawEquals = true;
                        this.emit(runStart, runStart + eq, "flag");
                        const value = run.slice(eq + 1);
                        const kind = classifyArg(value);
                        this.emit(runStart + eq + 1, this.i, kind === "plain" ? "flag-value" : kind);
                    } else if (sawEquals) {
                        this.emit(runStart, this.i, "flag-value");
                    } else {
                        this.emit(runStart, this.i, "flag");
                    }
                    continue;
                }
                // Plain argument. A subcommand is a leading word of a known program.
                const depthLimit = SUBCOMMAND_DEPTH[program] ?? 0;
                if (
                    atWordStart &&
                    subIdx < depthLimit &&
                    subIdx === wordIdx - 1 &&
                    SUBCOMMAND_RE.test(run) &&
                    (this.i >= n || isWordBreak(s[this.i]))
                ) {
                    this.emit(runStart, this.i, "subcommand");
                    subIdx++;
                } else {
                    const kind = classifyArg(run);
                    if (kind !== "plain") this.emit(runStart, this.i, kind);
                }
            }

            if (this.i === wordStart) {
                // Nothing consumed (a stray break char): never loop.
                this.i++;
                continue;
            }
            if (role === "env-value" || role === "wrapper-value") cmdPos = true;
            else if (role === "keyword") cmdPos = keywordThenCommand;
            else if (role === "program") {
                if (!programEmitted) {
                    program = "";
                    wrapper = false;
                }
                cmdPos = wrapper;
            } else if (role === "flag" && wrapper) cmdPos = true;
            else cmdPos = false;
            if ((role === "arg" || role === "flag") && !wrapper && wordIdx > 0) wordIdx++;
        }
        if (top && depth === 0) this.closeSegment(n);
    }

    private scanSingle(): void {
        const s = this.s;
        const start = this.i;
        const close = s.indexOf("'", this.i + 1);
        this.i = close < 0 ? s.length : close + 1;
        this.emit(start, this.i, "string");
    }

    private scanDouble(): void {
        const s = this.s;
        const n = s.length;
        let runStart = this.i;
        this.i++;
        while (this.i < n) {
            const c = s[this.i];
            if (c === "\\") {
                this.i += 2;
                continue;
            }
            if (c === '"') {
                this.i++;
                this.emit(runStart, Math.min(this.i, n), "string");
                return;
            }
            if (c === "$" && this.dollarLength(this.i) > 0) {
                this.emit(runStart, this.i, "string");
                this.tryDollar();
                runStart = this.i;
                continue;
            }
            if (c === "`") {
                this.emit(runStart, this.i, "string");
                this.scanBacktick();
                runStart = this.i;
                continue;
            }
            this.i++;
        }
        this.i = n;
        this.emit(runStart, n, "string");
    }

    private scanBacktick(): void {
        const s = this.s;
        if (this.nest >= MAX_NEST) {
            this.i++;
            return;
        }
        this.emit(this.i, this.i + 1, "substitution");
        this.i++;
        this.scanList("`");
        if (s[this.i] === "`") {
            this.emit(this.i, this.i + 1, "substitution");
            this.i++;
        }
    }

    /**
     * `$((expr))` and `((expr))`: the body is arithmetic, not shell words, so
     * `<<`, `<` and `&` in it are not heredocs, redirects or operators. Only
     * variables inside are marked.
     */
    private scanArithmetic(openLen: number): void {
        const s = this.s;
        this.emit(this.i, this.i + openLen, "substitution");
        this.i += openLen;
        let depth = 0;
        while (this.i < s.length) {
            const c = s[this.i];
            if (c === "$" && this.tryDollar()) continue;
            if (c === "(") depth++;
            else if (c === ")") {
                if (depth === 0) {
                    if (s[this.i + 1] === ")") {
                        this.emit(this.i, this.i + 2, "substitution");
                        this.i += 2;
                    } else {
                        this.i++;
                    }
                    return;
                }
                depth--;
            }
            this.i++;
        }
    }

    /** Length of the `$` construct at `at`, or 0 when it is just a dollar sign. */
    private dollarLength(at: number): number {
        const s = this.s;
        const next = s[at + 1];
        if (next === "(") return this.nest < MAX_NEST ? 2 : 0;
        if (next === "{") return 2;
        if (next === "'") return 2;
        VARIABLE_RE.lastIndex = at;
        const m = VARIABLE_RE.exec(s);
        return m ? m[0].length : 0;
    }

    /** Consume the `$` construct at `i`; false (nothing consumed) for a bare `$`. */
    private tryDollar(): boolean {
        const s = this.s;
        const len = this.dollarLength(this.i);
        if (len === 0) return false;
        const next = s[this.i + 1];
        if (next === "(" && s[this.i + 2] === "(") {
            this.scanArithmetic(3);
            return true;
        }
        if (next === "(") {
            this.emit(this.i, this.i + 2, "substitution");
            this.i += 2;
            this.scanList(")");
            if (s[this.i] === ")") {
                this.emit(this.i, this.i + 1, "substitution");
                this.i++;
            }
            return true;
        }
        if (next === "{") {
            let depth = 0;
            let j = this.i + 1;
            for (; j < s.length; j++) {
                if (s[j] === "{") depth++;
                else if (s[j] === "}" && --depth === 0) break;
            }
            const end = Math.min(j + 1, s.length);
            this.emit(this.i, end, "variable");
            this.i = end;
            return true;
        }
        if (next === "'") {
            const start = this.i;
            this.i += 2;
            while (this.i < s.length && s[this.i] !== "'") this.i += s[this.i] === "\\" ? 2 : 1;
            this.i = Math.min(this.i + 1, s.length);
            this.emit(start, this.i, "string");
            return true;
        }
        this.emit(this.i, this.i + len, "variable");
        this.i += len;
        return true;
    }
}

/** Make the tokens tile `[0, total)` in order, filling gaps with `plain`. */
function normalize(raw: Token[], total: number): Token[] {
    const out: Token[] = [];
    let pos = 0;
    const push = (start: number, end: number, kind: TokenKind): void => {
        if (end <= start) return;
        const last = out[out.length - 1];
        if (last && last.kind === kind && kind === "plain" && last.end === start) last.end = end;
        else out.push({ start, end, kind });
    };
    for (const t of raw) {
        const start = Math.max(t.start, pos);
        const end = Math.min(t.end, total);
        if (end <= start) continue;
        push(pos, start, "plain");
        push(start, end, t.kind);
        pos = end;
    }
    push(pos, total, "plain");
    return out;
}

/** Tokenize a POSIX shell command. Never throws; tokens always tile the input. */
export function tokenizeShell(input: string): Tokenized {
    const text = input.length > MAX_TOKENIZE_CHARS ? input.slice(0, MAX_TOKENIZE_CHARS) : input;
    let raw: Token[] = [];
    let segments: Segment[] = [];
    try {
        const scanner = new Scanner(text);
        scanner.scanList(null);
        raw = scanner.out;
        segments = scanner.segments;
    } catch {
        raw = [];
        segments = [];
    }
    return { tokens: normalize(raw, input.length), segments };
}
