// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BashCommandView, ShellCommand } from "./ShellCommand";

// The PowerShell path hands off to Shiki; keep that out of these tests.
vi.mock("../HighlightedCode", () => ({
    HighlightedCode: (p: { code: string; lang: string }) => <pre data-lang={p.lang}>{p.code}</pre>,
}));

afterEach(() => cleanup());

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
