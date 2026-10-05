// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { SHELL_LANG, embeddedBodies } from "./embedded";
import { tokenizeShell } from "./tokenize";

function langs(command: string): string[] {
    return embeddedBodies(tokenizeShell(command).tokens, command).map((b) => b.lang);
}

describe("embeddedBodies", () => {
    it("takes the language from the program that reads the body", () => {
        expect(langs("python - <<'PY'\nimport io\nPY")).toEqual(["python"]);
        expect(langs("node <<'EOF'\nconsole.log(1)\nEOF")).toEqual(["javascript"]);
        expect(langs("psql -d app <<SQL\nselect 1;\nSQL")).toEqual(["sql"]);
        expect(langs("bash <<'EOF'\nls -la && echo hi\nEOF")).toEqual([SHELL_LANG]);
    });

    it("takes the language from the file the body is written to", () => {
        const cmd = "cd /tmp/x && cat >> frontend/layout/tests/layoutResize.test.ts << 'EOF'\nconst a = 1;\nEOF";
        expect(langs(cmd)).toEqual(["typescript"]);
        expect(langs("cat > notes.md <<'EOF'\n# Notes\nEOF")).toEqual(["markdown"]);
        expect(langs("cat > run.sh <<'EOF'\necho hi\nEOF")).toEqual([SHELL_LANG]);
    });

    it("uses tee's operand, not where tee's own output is redirected", () => {
        expect(langs(`tee out.json >/dev/null <<'EOF'\n{"a":1}\nEOF`)).toEqual(["json"]);
        expect(langs("tee -a config.yaml <<'EOF'\nkey: v\nEOF")).toEqual(["yaml"]);
    });

    it("treats commit messages and PR bodies as markdown", () => {
        expect(langs("git commit -F - <<'MSG'\nfix: x\nMSG")).toEqual(["markdown"]);
        expect(langs(`git commit -m "$(cat <<'EOF'\nfix: x\nEOF\n)"`)).toEqual(["markdown"]);
        expect(langs("gh-agent pr create --body-file - <<'EOF'\n## Summary\nEOF")).toEqual(["markdown"]);
    });

    it("falls back to the delimiter's name", () => {
        expect(langs("cat <<'JSON'\n{}\nJSON")).toEqual(["json"]);
    });

    it("falls back to what the heredoc is piped into", () => {
        expect(langs("cat <<EOF | kubectl apply -f -\napiVersion: v1\nEOF")).toEqual(["yaml"]);
        expect(langs("cat <<'EOF' | python3\nprint(1)\nEOF")).toEqual(["python"]);
    });

    it("falls back to a shebang", () => {
        expect(langs("cat > script <<'EOF'\n#!/usr/bin/env python3\nprint(1)\nEOF")).toEqual(["python"]);
    });

    it("leaves a body of unknown language out", () => {
        expect(langs("cat <<'EOF'\nhello\nEOF")).toEqual([]);
        expect(langs("cat > notes.txt <<'EOF'\nhello\nEOF")).toEqual([]);
    });

    it("pairs each body with its own opener", () => {
        const cmd = "python - <<'PY'\nprint(1)\nPY\ncat > a.json <<'EOF'\n{}\nEOF";
        const bodies = embeddedBodies(tokenizeShell(cmd).tokens, cmd);
        expect(bodies.map((b) => [cmd.slice(b.start, b.end), b.lang])).toEqual([
            ["print(1)\n", "python"],
            ["{}\n", "json"],
        ]);
    });
});
