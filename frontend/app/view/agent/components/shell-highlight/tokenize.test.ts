// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { MAX_TOKENIZE_CHARS, tokenizeShell, type TokenKind } from "./tokenize";

/** Non-plain tokens as `[text, kind]` pairs. */
function marks(command: string): [string, TokenKind][] {
    return tokenizeShell(command)
        .tokens.filter((t) => t.kind !== "plain")
        .map((t) => [command.slice(t.start, t.end), t.kind]);
}

function roundTrip(command: string): string {
    return tokenizeShell(command)
        .tokens.map((t) => command.slice(t.start, t.end))
        .join("");
}

describe("tokenizeShell kinds", () => {
    it("marks program, subcommand, flags and paths", () => {
        expect(marks("git commit -m msg ./a/b.ts")).toEqual([
            ["git", "program"],
            ["commit", "subcommand"],
            ["-m", "flag"],
            ["./a/b.ts", "path"],
        ]);
    });

    it("takes two subcommand words for gh and docker", () => {
        expect(marks("gh pr view 12")).toEqual([
            ["gh", "program"],
            ["pr", "subcommand"],
            ["view", "subcommand"],
            ["12", "number"],
        ]);
    });

    it("does not treat a later word as a subcommand once an argument came first", () => {
        expect(marks("git -C repo status")).toEqual([
            ["git", "program"],
            ["-C", "flag"],
        ]);
    });

    it("splits --flag=value", () => {
        expect(marks("cargo build --target=x86_64-pc-windows-msvc --out=./dist")).toEqual([
            ["cargo", "program"],
            ["build", "subcommand"],
            ["--target", "flag"],
            ["x86_64-pc-windows-msvc", "flag-value"],
            ["--out", "flag"],
            ["./dist", "path"],
        ]);
    });

    it("marks pipes, chains and the program after each", () => {
        expect(marks("cd x && ls | wc -l; echo done")).toEqual([
            ["cd", "program"],
            ["&&", "operator"],
            ["ls", "program"],
            ["|", "operator"],
            ["wc", "program"],
            ["-l", "flag"],
            [";", "operator"],
            ["echo", "program"],
        ]);
    });

    it("marks quotes, variables and substitutions", () => {
        expect(marks(`echo "hi $USER" '$x' $(date +%s) \${HOME}/bin`)).toEqual([
            ["echo", "program"],
            ['"hi ', "string"],
            ["$USER", "variable"],
            ['"', "string"],
            ["'$x'", "string"],
            ["$(", "substitution"],
            ["date", "program"],
            [")", "substitution"],
            ["${HOME}", "variable"],
            ["/bin", "path"],
        ]);
    });

    it("marks redirects, including fd duplication", () => {
        expect(marks("make 2>&1 > out.log")).toEqual([
            ["make", "program"],
            ["2>&1", "redirect"],
            [">", "redirect"],
        ]);
        expect(marks("cmd &> all.txt")).toContainEqual(["&>", "redirect"]);
        expect(marks("sort < in.txt >> out.txt")).toEqual([
            ["sort", "program"],
            ["<", "redirect"],
            [">>", "redirect"],
        ]);
    });

    it("marks environment assignments and keeps the program after them", () => {
        expect(marks("FOO=1 BAR=x npm test")).toEqual([
            ["FOO=", "env-assign"],
            ["BAR=", "env-assign"],
            ["npm", "program"],
            ["test", "subcommand"],
        ]);
    });

    it("treats the word after sudo/env as a program", () => {
        expect(marks("sudo -n rm file")).toEqual([
            ["sudo", "program"],
            ["-n", "flag"],
            ["rm", "program"],
        ]);
    });

    it("skips the value of a wrapper option before the wrapped program", () => {
        const programs = (cmd: string) =>
            marks(cmd)
                .filter(([, k]) => k === "program")
                .map(([t]) => t);
        expect(programs("sudo -u root rm file")).toEqual(["sudo", "rm"]);
        expect(programs("sudo -u root -E make")).toEqual(["sudo", "make"]);
        expect(programs('sudo --user "root" ls')).toEqual(["sudo", "ls"]);
        expect(programs("sudo --user=root ls")).toEqual(["sudo", "ls"]);
        expect(programs("sudo -uroot ls")).toEqual(["sudo", "ls"]);
        expect(programs("env -u NAME node x.js")).toEqual(["env", "node"]);
        expect(programs("nice -n 5 make")).toEqual(["nice", "make"]);
        expect(marks("nice -n 5 make")).toContainEqual(["5", "number"]);
    });

    it("treats time as a wrapper with options", () => {
        const programs = (cmd: string) =>
            marks(cmd)
                .filter(([, k]) => k === "program")
                .map(([t]) => t);
        expect(programs("time -p sleep 1")).toEqual(["time", "sleep"]);
        expect(programs("time -f '%e' make")).toEqual(["time", "make"]);
        expect(programs("time cargo build")).toEqual(["time", "cargo"]);
    });

    it("reads case patterns as patterns and the arm as a command", () => {
        const cmd = 'case "$x" in a|b) echo yes;; c) ls -l;& *) rm f;;& esac; pwd';
        const m = marks(cmd);
        expect(m.filter(([, k]) => k === "program").map(([t]) => t)).toEqual(["echo", "ls", "rm", "pwd"]);
        expect(m.filter(([, k]) => k === "keyword").map(([t]) => t)).toEqual(["case", "in", "esac"]);
        expect(roundTrip(cmd)).toBe(cmd);
    });

    it("handles a multi-line case with a last arm without ;; and a case inside $( )", () => {
        const cmd = 'case $1 in\n  (start) run --x\n    ;;\n  stop) halt\nesac\nv=$(case $y in a) echo 1;; esac)\nls';
        const m = marks(cmd);
        expect(m.filter(([, k]) => k === "program").map(([t]) => t)).toEqual(["run", "halt", "echo", "ls"]);
        expect(roundTrip(cmd)).toBe(cmd);
    });

    it("marks keywords and the command after them", () => {
        expect(marks("if [ -f x ]; then echo y; fi")).toEqual([
            ["if", "keyword"],
            ["[", "program"],
            ["-f", "flag"],
            [";", "operator"],
            ["then", "keyword"],
            ["echo", "program"],
            [";", "operator"],
            ["fi", "keyword"],
        ]);
    });

    it("marks comments only at a word start", () => {
        expect(marks("ls # list\necho a#b")).toEqual([
            ["ls", "program"],
            ["# list", "comment"],
            ["echo", "program"],
        ]);
    });

    it("marks urls and windows paths", () => {
        expect(marks("curl https://example.com/x C:\\Users\\a\\b.txt")).toEqual([
            ["curl", "program"],
            ["https://example.com/x", "url"],
            ["C:\\Users\\a\\b.txt", "path"],
        ]);
    });
});

describe("tokenizeShell heredocs", () => {
    it("marks the marker, the body and the terminator", () => {
        const cmd = "cat > f.txt <<'EOF'\nline $one\nEOF\necho after";
        expect(marks(cmd)).toEqual([
            ["cat", "program"],
            [">", "redirect"],
            ["<<'EOF'", "heredoc-marker"],
            ["line $one\n", "heredoc-body"],
            ["EOF", "heredoc-marker"],
            ["echo", "program"],
        ]);
    });

    it("handles a heredoc inside $(...) inside a string (git commit idiom)", () => {
        const cmd = 'git commit -m "$(cat <<\'EOF\'\nsubject\n\nbody line\nEOF\n)"';
        const m = marks(cmd);
        expect(m).toContainEqual(["subject\n\nbody line\n", "heredoc-body"]);
        expect(m).toContainEqual(["EOF", "heredoc-marker"]);
        // The closing `)` and quote still pair up after the body.
        expect(m[m.length - 1]).toEqual(['"', "string"]);
    });

    it("supports <<- with tab-indented terminator and several heredocs on one line", () => {
        const cmd = "cat <<-A <<B\n\tone\n\tA\ntwo\nB\n";
        const m = marks(cmd);
        expect(m.filter(([, k]) => k === "heredoc-body").map(([t]) => t)).toEqual(["\tone\n", "two\n"]);
    });

    it("treats an unterminated heredoc body as running to the end", () => {
        const cmd = "cat <<EOF\nno end";
        expect(marks(cmd)).toContainEqual(["no end", "heredoc-body"]);
        expect(roundTrip(cmd)).toBe(cmd);
    });

    it("does not read a here-string as a heredoc", () => {
        expect(marks("grep x <<< hello").filter(([, k]) => k === "heredoc-marker")).toEqual([]);
    });
});

describe("tokenizeShell arithmetic", () => {
    it("does not read << inside $(( )) as a heredoc", () => {
        const cmd = "x=$((1 << 2))\necho hi\nls";
        const m = marks(cmd);
        expect(m.filter(([, k]) => k === "heredoc-marker" || k === "heredoc-body")).toEqual([]);
        expect(m).toContainEqual(["echo", "program"]);
        expect(m).toContainEqual(["ls", "program"]);
    });

    it("marks variables inside arithmetic and keeps redirects out of it", () => {
        expect(marks("echo $((a<<2 & $b)) && ls")).toEqual([
            ["echo", "program"],
            ["$((", "substitution"],
            ["$b", "variable"],
            ["))", "substitution"],
            ["&&", "operator"],
            ["ls", "program"],
        ]);
    });

    it("handles the (( )) command form and nested parentheses", () => {
        const cmd = "((i = (1 << 2) + 3))\nls";
        const m = marks(cmd);
        expect(m.filter(([, k]) => k === "heredoc-marker")).toEqual([]);
        expect(m).toContainEqual(["ls", "program"]);
        expect(roundTrip(cmd)).toBe(cmd);
    });
});

describe("tokenizeShell segments", () => {
    it("splits at operators and newlines, with the program of each", () => {
        const cmd = "cd a && make -j4 | tee log\nls";
        const { segments } = tokenizeShell(cmd);
        expect(segments.map((s) => cmd.slice(s.start, s.end).trim())).toEqual(["cd a", "make -j4", "tee log", "ls"]);
        expect(segments.map((s) => (s.program ? cmd.slice(s.program.start, s.program.end) : null))).toEqual([
            "cd",
            "make",
            "tee",
            "ls",
        ]);
    });

    it("does not split inside a substitution or subshell", () => {
        const cmd = "echo $(a | b) && (c; d)";
        const { segments } = tokenizeShell(cmd);
        expect(segments.map((s) => cmd.slice(s.start, s.end).trim())).toEqual(["echo $(a | b)", "(c; d)"]);
    });
});

describe("tokenizeShell robustness", () => {
    const corpus = [
        "",
        " ",
        "ls",
        "ls -la /tmp",
        "cd /d C:\\x && dir",
        "echo 'unterminated",
        'echo "unterminated $(date',
        "echo $(echo $(echo $(echo hi)))",
        "echo `date`",
        "echo `echo \\`nested\\``",
        "a | b |& c || d && e ; f & g",
        "x=$((1 + 2))",
        "for f in *.ts; do wc -l $f; done",
        "case $x in a) echo a;; esac",
        "<(ls) >(cat)",
        "cat <<EOF",
        "echo \\",
        "echo a\\\nb",
        "$",
        "${",
        "${a:-${b}}",
        "$'a\\'b'",
        ")))((( ",
        "\r\n\r\n",
        "echo é 日本語 🚀",
        "\u0000\u0001binary\u007f",
    ];

    it("tiles the input exactly for the corpus", () => {
        for (const cmd of corpus) expect(roundTrip(cmd)).toBe(cmd);
    });

    it("tiles the input exactly for random input", () => {
        const alphabet = ["a", "b", " ", "\n", "'", '"', "$", "(", ")", "`", "|", "&", ";", "<", ">", "\\", "#", "=", "-", "{", "}", "0", "EOF"];
        let seed = 12345;
        const rnd = () => {
            seed = (seed * 1103515245 + 12345) & 0x7fffffff;
            return seed;
        };
        for (let n = 0; n < 500; n++) {
            let cmd = "";
            const len = rnd() % 60;
            for (let k = 0; k < len; k++) cmd += alphabet[rnd() % alphabet.length];
            const { tokens } = tokenizeShell(cmd);
            expect(tokens.map((t) => cmd.slice(t.start, t.end)).join("")).toBe(cmd);
            let pos = 0;
            for (const t of tokens) {
                expect(t.start).toBe(pos);
                expect(t.end).toBeGreaterThan(t.start);
                pos = t.end;
            }
            expect(pos).toBe(cmd.length);
        }
    });

    it("survives pathological nesting", () => {
        const cmd = "$(".repeat(5000) + "x" + ")".repeat(5000);
        expect(roundTrip(cmd)).toBe(cmd);
        const ticks = "`".repeat(5000);
        expect(roundTrip(ticks)).toBe(ticks);
    });

    it("caps very long commands and returns the rest as plain text", () => {
        const cmd = "echo " + "a".repeat(MAX_TOKENIZE_CHARS * 2);
        const { tokens } = tokenizeShell(cmd);
        expect(roundTrip(cmd)).toBe(cmd);
        const last = tokens[tokens.length - 1];
        expect(last.end).toBe(cmd.length);
    });
});
