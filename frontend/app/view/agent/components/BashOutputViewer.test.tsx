// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";
import type { BashResult } from "../types";
import { BashOutputViewer } from "./BashOutputViewer";

afterEach(() => cleanup());

// SPEC_AGENT_PANE_PREVIEW_CLEANUPS_2026_09_26.md §4: a destructured prop is
// frozen at mount, so a viewer kept mounted across a result update would
// show the first result forever (the hazard #3877 fixed in CompactResult).
describe("BashOutputViewer", () => {
    it("follows a result that changes while mounted", () => {
        const [result, setResult] = createSignal<BashResult | undefined>(undefined);
        const { container } = render(() => <BashOutputViewer result={result()} />);
        expect(container.querySelector(".agent-bash-output")).toBeNull();

        setResult({ stdout: "<exited 1 in 2.00s>\nfirst failure", stderr: "", exitCode: undefined as any });
        expect(container.querySelector(".agent-bash-output")!.textContent).toBe("first failure");
        expect(container.querySelector(".agent-bash-exit")!.textContent).toBe("Exit code: 1");

        setResult({ stdout: "<exited 0 in 1.00s>\nall green", stderr: "", exitCode: undefined as any });
        expect(container.querySelector(".agent-bash-output")!.textContent).toBe("all green");
        expect(container.querySelector(".agent-bash-exit")!.textContent).toBe("Exit code: 0");
    });

    it("strips the bashwrap prefix and shows stderr after stdout, marked", () => {
        const { container } = render(() => (
            <BashOutputViewer result={{ stdout: "<exited 2 in 0.10s>\nout", stderr: "err", exitCode: undefined as any }} />
        ));
        const lines = [...container.querySelectorAll(".agent-bash-output .agent-preview-line")];
        expect(lines.map((l) => l.textContent)).toEqual(["out", "err"]);
        expect(lines.map((l) => l.classList.contains("agent-preview-line--stderr"))).toEqual([false, true]);
        expect(container.querySelector(".agent-bash-exit.exit-error")).not.toBeNull();
    });

    it("prints the output, not the command: the tool row and its hover show that", () => {
        const { container } = render(() => <BashOutputViewer result={{ stdout: "built ok", stderr: "", exitCode: 0 } as any} />);
        expect(container.querySelector(".agent-bash-cmd")).toBeNull();
        expect(container.querySelector(".agent-bash-output")!.textContent).toBe("built ok");
    });

    it("drops the cursor report bashwrap's PTY echoed into results recorded before #4508", () => {
        const { container } = render(() => (
            <BashOutputViewer result={{ stdout: "<exited 0 in 0.02s>\n^[[1;1Rhello", stderr: "" } as BashResult} />
        ));
        expect(container.querySelector(".agent-bash-output")!.textContent).toBe("hello");
    });

    it("says No output when the call printed nothing", () => {
        const { container } = render(() => <BashOutputViewer result={{ stdout: "", stderr: "", exitCode: 0 } as any} />);
        expect(container.querySelector(".agent-bash-no-output")!.textContent).toBe("No output");
        expect(container.querySelector(".agent-bash-exit")!.textContent).toBe("Exit code: 0");
    });
});
