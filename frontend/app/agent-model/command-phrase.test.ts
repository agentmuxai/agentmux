// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { phraseCommand } from "./command-phrase";

describe("phraseCommand", () => {
    it("skips a leading cd and names the folder it went to", () => {
        expect(phraseCommand("cd ~/.agentmux/agents/maricon-09101/work/rr && cargo test -p srv")).toBe(
            "Run cargo test in rr"
        );
    });

    it("names the real program through a chain, an environment prefix and wrappers", () => {
        expect(phraseCommand("cd /repo/app && FOO=1 BAR=2 npm run build")).toBe("Build with npm in app");
        expect(phraseCommand("FORCE_COLOR=1 sudo -E timeout 30 cargo check --tests")).toBe("Run cargo check");
        expect(phraseCommand("export A=b; source ./env.sh; git status")).toBe("Run git status");
    });

    it("sees through secrets escalate and gh-agent sudo", () => {
        expect(phraseCommand('eval "$(secrets escalate debug --reason "look" -- aws logs tail)"')).toBe("Run aws logs");
        expect(phraseCommand("secrets escalate admin --reason x -- aws s3 ls")).toBe("Run aws s3");
        expect(phraseCommand("gh-agent sudo --reason x --repo a/b -- gh api repos")).toBe("Run gh api");
    });

    it("reads the command string given to a shell", () => {
        expect(phraseCommand('bash -c "cd /work/rr && cargo build"')).toBe("Run cargo build in rr");
        expect(phraseCommand('powershell -Command "cd C:\\Users\\yas\\proj; npm test"')).toBe(
            "Run tests with npm in proj"
        );
    });

    it("names a script by its file, not its path", () => {
        expect(phraseCommand("node scripts/check-function-sizes.mjs --update")).toBe("Run check-function-sizes");
        expect(phraseCommand("python3 /home/yas/work/rr/probe.py")).toBe("Run probe");
        expect(phraseCommand("bash ../run-checks-pr3.sh")).toBe("Run run-checks-pr3");
    });

    it("says everyday programs plainly", () => {
        expect(phraseCommand("ls -la ~/.agentmux/agents/agent1-06309")).toBe("List files");
        expect(phraseCommand("grep -rn foo src | head -5")).toBe("Search files");
        expect(phraseCommand("sleep 30")).toBe("Wait");
    });

    it("never shows the home path or an agent workspace", () => {
        expect(phraseCommand("cd ~ && cargo build")).toBe("Run cargo build");
        expect(phraseCommand("cd ~/.agentmux/agents/agent1-06309 && git status")).toBe("Run git status");
        expect(phraseCommand("cd C:\\Users\\yas && dir")).toBe("List files");
        expect(phraseCommand("cd /home/yas/.agentmux/agents/maricon-09101/work/rr ; make test")).toBe(
            "Run make test in rr"
        );
    });

    it("handles a Windows path and cd /d", () => {
        expect(phraseCommand("cd /d C:\\work\\rr && cargo clippy")).toBe("Run cargo clippy in rr");
        expect(phraseCommand("C:\\tools\\node.exe C:\\work\\rr\\build.mjs")).toBe("Run build");
    });

    it("keeps a very long command to one short phrase", () => {
        const long = `cd /a/b && ${"verylongprogramname ".repeat(40)}`;
        expect(phraseCommand(long).length).toBeLessThanOrEqual(80);
        expect(phraseCommand(`npm run ${"x".repeat(200)}`).length).toBeLessThanOrEqual(80);
    });

    it("names no script for an interpreter given inline code, a heredoc or standard input", () => {
        expect(phraseCommand("python - <<'PY'\nprint(1)\nPY")).toBe("Run python");
        expect(phraseCommand('node -e "console.log(1)"')).toBe("Run node");
        expect(phraseCommand("python3 -c 'import os'")).toBe("Run python3");
        expect(phraseCommand("python -m pytest -x")).toBe("Run pytest");
    });

    it("reads an assignment with a quoted value as one word", () => {
        expect(phraseCommand('FOO="a b" cargo test')).toBe("Run cargo test");
        expect(phraseCommand("FOO='x y' BAR=\"1 2\" git status")).toBe("Run git status");
    });

    it("looks past shell syntax: keywords, subshells, negation and redirections", () => {
        expect(phraseCommand("if [ -f x ]; then echo hi; fi")).toBe("Print text");
        expect(phraseCommand("for f in *.ts; do echo $f; done")).toBe("Print text");
        expect(phraseCommand("2>&1 cargo test")).toBe("Run cargo test");
        expect(phraseCommand("> out.txt echo hi")).toBe("Print text");
        expect(phraseCommand("! git diff --quiet")).toBe("Run git diff");
        expect(phraseCommand("(cd foo && make)")).toBe("Run make in foo");
    });

    it("never takes the value of a flag for the subcommand, the program or the folder", () => {
        expect(phraseCommand("kubectl -n prod get pods")).toBe("Run kubectl get");
        expect(phraseCommand("make -C sub test")).toBe("Run make test");
        expect(phraseCommand("npm --prefix foo run build")).toBe("Build with npm");
        expect(phraseCommand("sudo -u bob ls")).toBe("List files");
    });

    it("falls back to a plain phrase when nothing can be named", () => {
        expect(phraseCommand("")).toBe("Run a shell command");
        expect(phraseCommand(undefined)).toBe("Run a shell command");
        expect(phraseCommand("cd /tmp && export A=1")).toBe("Run a shell command");
    });
});
