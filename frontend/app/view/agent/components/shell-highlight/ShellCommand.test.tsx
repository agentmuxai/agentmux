// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { highlightBody } from "./embedded-highlight";
import { BashCommandView, ShellCommand } from "./ShellCommand";

// The PowerShell path hands off to Shiki; keep that out of these tests.
vi.mock("../HighlightedCode", () => ({
    HighlightedCode: (p: { code: string; lang: string }) => <pre data-lang={p.lang}>{p.code}</pre>,
}));

// Shiki stands in as one coloured run per word; JSON fails.
vi.mock("./embedded-highlight", () => ({
    cachedRuns: () => undefined,
    highlightBody: vi.fn(async (text: string, lang: string) =>
        lang === "json"
            ? null
            : text.split(/(\s+)/).map((w) => (/\S/.test(w) ? { text: w, style: { "--shiki-dark": "#f00" } } : { text: w }))
    ),
}));

afterEach(() => {
    cleanup();
    vi.mocked(highlightBody).mockClear();
});

describe("ShellCommand", () => {
    it("is coloured on the first render, with no async step", () => {
        const { container } = render(() => <ShellCommand command="git commit -m 'x' | tee out.log" />);
        expect(container.querySelector(".sh-program")!.textContent).toBe("git");
        expect(container.querySelector(".sh-subcommand")!.textContent).toBe("commit");
        expect(container.querySelector(".sh-flag")!.textContent).toBe("-m");
        expect(container.querySelector(".sh-string")!.textContent).toBe("'x'");
        expect(container.querySelector(".sh-operator")!.textContent).toBe("|");
    });

    it("never changes the text", () => {
        const cmd = 'cat > f.ts <<\'EOF\'\nconst a = "$x";\nEOF\necho "$(date)" # done';
        const { container } = render(() => <ShellCommand command={cmd} />);
        expect(container.querySelector("pre")!.textContent).toBe(cmd);
    });

    it("follows a command that changes while mounted", () => {
        const [cmd, setCmd] = createSignal("ls -la");
        const { container } = render(() => <ShellCommand command={cmd()} />);
        expect(container.querySelector(".sh-program")!.textContent).toBe("ls");
        setCmd("cargo build --release");
        expect(container.querySelector(".sh-program")!.textContent).toBe("cargo");
        expect(container.querySelector(".sh-subcommand")!.textContent).toBe("build");
        expect(container.querySelector("pre")!.textContent).toBe("cargo build --release");
    });

    it("hands PowerShell to the PowerShell grammar", () => {
        const { container } = render(() => <ShellCommand command="Get-ChildItem | Select-Object Name" />);
        expect(container.querySelector("pre")!.getAttribute("data-lang")).toBe("powershell");
    });

    it("shows cmd as plain text", () => {
        const { container } = render(() => <ShellCommand command="echo %USERPROFILE%" />);
        expect(container.querySelector(".sh-program")).toBeNull();
        expect(container.querySelector("pre")!.textContent).toBe("echo %USERPROFILE%");
    });

    it("colours a shell heredoc body on the first render", () => {
        const cmd = "bash <<'EOF'\nls -la | wc -l\nEOF";
        const { container } = render(() => <ShellCommand command={cmd} />);
        const body = container.querySelector(".sh-embedded")!;
        expect(body.textContent).toBe("ls -la | wc -l\n");
        expect(body.querySelector(".sh-program")!.textContent).toBe("ls");
        expect(container.querySelector("pre")!.textContent).toBe(cmd);
    });

    it("upgrades a body in another language to Shiki colours without changing the text", async () => {
        const cmd = "cat > a.ts <<'EOF'\nconst a = 1;\nEOF";
        const { container } = render(() => <ShellCommand command={cmd} />);
        // First paint: the body as plain text.
        expect(container.querySelector(".sh-heredoc-body")!.textContent).toBe("const a = 1;\n");
        await waitFor(() => expect(container.querySelector(".sh-shiki")).not.toBeNull());
        const run = container.querySelector<HTMLElement>(".sh-shiki span")!;
        expect(run.textContent).toBe("const");
        expect(run.style.getPropertyValue("--shiki-dark")).toBe("#f00");
        expect(highlightBody).toHaveBeenCalledWith("const a = 1;\n", "typescript");
        expect(container.querySelector("pre")!.textContent).toBe(cmd);
    });

    it("keeps a body plain when Shiki can't highlight it", async () => {
        const cmd = "cat > a.json <<'EOF'\n{}\nEOF";
        const { container } = render(() => <ShellCommand command={cmd} />);
        await Promise.resolve();
        expect(container.querySelector(".sh-shiki")).toBeNull();
        expect(container.querySelector(".sh-heredoc-body")!.textContent).toBe("{}\n");
    });

    it("leaves a body of unknown language plain, without asking Shiki", () => {
        const { container } = render(() => <ShellCommand command={"cat <<'EOF'\nhello\nEOF"} />);
        expect(container.querySelector(".sh-heredoc-body")!.textContent).toBe("hello\n");
        expect(highlightBody).not.toHaveBeenCalled();
    });

    it("tolerates a missing command", () => {
        const { container } = render(() => <ShellCommand command={undefined as unknown as string} />);
        expect(container.querySelector("pre")!.textContent).toBe("");
    });
});

describe("BashCommandView", () => {
    it("renders the dollar prompt and the command", () => {
        const { container } = render(() => <BashCommandView command="npm test" />);
        expect(container.querySelector(".agent-bash-dollar")!.textContent).toBe("$");
        expect(container.querySelector(".agent-bash-cmd-code")!.textContent).toBe("npm test");
    });
});
