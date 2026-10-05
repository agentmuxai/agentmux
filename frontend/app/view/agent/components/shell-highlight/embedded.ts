// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which language a heredoc body is written in, so it can be highlighted as
 * that language instead of one flat string.
 *
 * In order (spec §3.2):
 *   1. the program that reads the body: `python - <<'PY'`, `node <<EOF`,
 *      `psql <<SQL`, `bash <<EOF`;
 *   2. the file it is written to: `cat > a.ts <<'EOF'`, `tee x.json <<EOF`;
 *   3. the delimiter's name: `<<'PY'`, `<<JSON`;
 *   4. what the heredoc is piped into: `cat <<EOF | kubectl apply -f -`;
 *   5. a shebang on the body's first line.
 * Anything else stays unknown and renders as plain text.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_BASH_HIGHLIGHTING_2026_10_04.md §3.2
 */

import { detectLanguage } from "../detectLanguage";
import type { Token } from "./tokenize";

/** The shell grammar is ours (the sync tokenizer), not Shiki's. */
export const SHELL_LANG = "shell";

export interface EmbeddedBody {
    /** The heredoc-body token's range in the command. */
    start: number;
    end: number;
    /** A Shiki language id, or SHELL_LANG. */
    lang: string;
}

/** Programs that read a heredoc from stdin, and the language they read. */
const READERS: Record<string, string> = {
    python: "python",
    python3: "python",
    py: "python",
    node: "javascript",
    deno: "typescript",
    bun: "typescript",
    tsx: "typescript",
    "ts-node": "typescript",
    bash: SHELL_LANG,
    sh: SHELL_LANG,
    zsh: SHELL_LANG,
    dash: SHELL_LANG,
    psql: "sql",
    sqlite3: "sql",
    mysql: "sql",
    ruby: "ruby",
    perl: "perl",
    php: "php",
    lua: "lua",
    pwsh: "powershell",
    powershell: "powershell",
    jq: "json",
    kubectl: "yaml",
};

/** Programs that write their stdin to the file they are given. */
const WRITERS = new Set(["cat", "tee"]);

/**
 * Programs whose heredocs are prose: commit messages and PR or issue bodies,
 * given directly (`gh pr create --body-file - <<EOF`) or through
 * `git commit -m "$(cat <<'EOF' ... EOF)"`. About a third of agents' heredocs.
 */
const PROSE = new Set(["git", "gh", "gh-agent", "gh-agent.sh", "glab"]);

/** Delimiter names that say what they hold. */
const DELIMITERS: Record<string, string> = {
    PY: "python",
    PYTHON: "python",
    JS: "javascript",
    TS: "typescript",
    JSON: "json",
    YAML: "yaml",
    YML: "yaml",
    SQL: "sql",
    MD: "markdown",
    SH: SHELL_LANG,
    BASH: SHELL_LANG,
    HTML: "html",
    CSS: "css",
    TOML: "toml",
    RS: "rust",
    RUST: "rust",
};

const SEPARATORS = new Set(["|", "|&", "&&", "||", ";", "&"]);

function unquote(text: string): string {
    return text.replace(/^(['"])([\s\S]*)\1$/, "$2");
}

function programName(text: string): string {
    const base = unquote(text).split(/[\\/]/).pop() ?? "";
    return base.toLowerCase().replace(/\.(exe|cmd|bat)$/, "");
}

/** `.sh` and friends come back from detectLanguage as Shiki's bash: use ours. */
function normalize(lang: string): string | null {
    if (!lang || lang === "text") return null;
    return lang === "bash" ? SHELL_LANG : lang;
}

/** A heredoc opener's delimiter: `<<'EOF'`, `<< "EOF"`, `<<-EOF`. */
function delimiterOf(opener: string): string {
    return unquote(opener.replace(/^<<-?\s*/, "").trim());
}

/**
 * Every heredoc body in `command` whose language can be told, with that
 * language. Bodies of an unknown language are left out.
 */
export function embeddedBodies(tokens: Token[], command: string): EmbeddedBody[] {
    const text = (t: Token) => command.slice(t.start, t.end);
    const out: EmbeddedBody[] = [];
    // Heredocs are read in the order their openers appeared: each body, and
    // then its closing delimiter, belongs to the oldest one still open. An
    // empty body has no token, only its closing delimiter, so pairing by
    // count would hand the next body to the wrong opener.
    const open: number[] = [];
    tokens.forEach((t, i) => {
        if (t.kind === "heredoc-marker") {
            if (text(t).startsWith("<<")) open.push(i);
            else open.shift();
        } else if (t.kind === "heredoc-body" && open.length) {
            const lang = languageFor(tokens, open[0], text, text(t), command);
            if (lang) out.push({ start: t.start, end: t.end, lang });
        }
    });
    return out;
}

/** True when `s` holds a line break that isn't a `\` line continuation. */
function hasLineBreak(s: string): boolean {
    return /(^|[^\\])\n/.test(s);
}

function languageFor(
    tokens: Token[],
    openerIdx: number,
    text: (t: Token) => string,
    body: string,
    command: string
): string | null {
    // The simple command the opener belongs to: back to the previous separator
    // or line break. The tokenizer emits no token for a newline: it sits in a
    // plain run or between tokens. One inside a quoted string doesn't count.
    let first = openerIdx;
    while (first > 0) {
        const prev = tokens[first - 1];
        if (prev.kind === "operator" && SEPARATORS.has(text(prev))) break;
        // A body or closing delimiter means an earlier line.
        if (prev.kind === "heredoc-body" || (prev.kind === "heredoc-marker" && !text(prev).startsWith("<<"))) break;
        if (hasLineBreak(command.slice(prev.end, tokens[first].start))) break;
        if (prev.kind === "plain" && hasLineBreak(text(prev))) break;
        first--;
    }
    // ... and forward to the end of the opener's line or the next separator.
    let last = openerIdx;
    while (
        last + 1 < tokens.length &&
        tokens[last + 1].kind !== "heredoc-body" &&
        !(tokens[last + 1].kind === "operator" && SEPARATORS.has(text(tokens[last + 1])))
    ) {
        last++;
    }

    // The program the heredoc feeds is the nearest one before it: in
    // `git commit -m "$(cat <<'EOF'`, that is `cat`; `git` encloses it.
    const programs = tokens.slice(first, openerIdx).filter((t) => t.kind === "program");
    const programTok = programs[programs.length - 1];
    const program = programTok ? programName(text(programTok)) : "";
    const enclosing = programs.slice(0, -1).map((t) => programName(text(t)));

    // 1. A program that reads the body.
    if (READERS[program]) return READERS[program];

    // 2. A file the body is written to.
    if (WRITERS.has(program)) {
        const target = writeTarget(tokens, programs.length ? tokens.indexOf(programTok) : first, last, program, text);
        const lang = target ? normalize(detectLanguage(target)) : null;
        if (lang) return lang;
    }

    // Prose: a commit message or a PR/issue body.
    if (PROSE.has(program) || (program === "cat" && enclosing.some((p) => PROSE.has(p)))) return "markdown";

    // 3. The delimiter's name.
    const delim = DELIMITERS[delimiterOf(text(tokens[openerIdx])).toUpperCase()];
    if (delim) return delim;

    // 4. What the heredoc is piped into (`cat <<EOF | python`).
    const after = tokens[last + 1];
    if (after && after.kind === "operator" && text(after) === "|") {
        const next = tokens.slice(last + 2).find((t) => t.kind === "program");
        if (next && READERS[programName(text(next))]) return READERS[programName(text(next))];
    }

    // 5. A shebang on the first line.
    const firstLine = body.split("\n", 1)[0] ?? "";
    return firstLine.startsWith("#!") ? normalize(detectLanguage("", firstLine)) : null;
}

/** The first word of the text from token `i` on, skipping leading whitespace. */
function wordAt(tokens: Token[], i: number, end: number, text: (t: Token) => string): string | null {
    for (let j = i; j <= end; j++) {
        const t = tokens[j];
        if (t.kind === "heredoc-marker" || t.kind === "redirect" || t.kind === "operator") return null;
        const word = text(t).trim().split(/\s+/)[0];
        if (word) return unquote(word);
    }
    return null;
}

/**
 * The file `cat > f`, `cat >> f` or `tee [-a] f` writes to. The tokenizer can
 * hand the name over as a `path` or as part of a `plain` run with spaces.
 */
function writeTarget(tokens: Token[], programIdx: number, last: number, program: string, text: (t: Token) => string): string | null {
    if (program === "tee") {
        // tee writes to its operands; a redirect after it (`>/dev/null`) is
        // only where tee's own copy of the input goes.
        for (let i = programIdx + 1; i <= last; i++) {
            const t = tokens[i];
            if (t.kind === "flag") continue;
            if (t.kind === "redirect" || t.kind === "heredoc-marker" || t.kind === "operator") break;
            const word = text(t).trim().split(/\s+/).find((w) => w && !w.startsWith("-"));
            if (word) return unquote(word);
        }
        return null;
    }
    for (let i = programIdx + 1; i <= last; i++) {
        const t = tokens[i];
        if (t.kind === "redirect" && /^\d*>>?$/.test(text(t))) {
            const target = wordAt(tokens, i + 1, last, text);
            if (target) return target;
        }
    }
    return null;
}
