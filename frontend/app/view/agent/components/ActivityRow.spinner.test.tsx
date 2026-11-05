// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A running dock row shows the animation of its kind of work to the left of its short phrase, once it has
 * run for 3 s (before that, and once it is over, its own glyph). The rotating slash is the turn's sign and
 * never appears here. SPEC_LONG_RUNNING_ACTIVITY_ANIMATIONS_2026_11_02.md §6.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";

import type { PinnedActivity } from "../../../agent-model/activity-types";
import type { ShellNode, ToolNode } from "../../../agent-model/types";
import { ActivityRow } from "./ActivityRow";
import { bucketOfActivity } from "./ActivitySigil";

afterEach(cleanup);

const bash = (command: string) => ({ tool: "Bash", toolName: "Bash", params: { command } }) as unknown as ToolNode;

const activity = (over: Partial<PinnedActivity> = {}): PinnedActivity => ({
    id: "t1",
    kind: "tool",
    title: "Wait for v0.60.10 to publish",
    status: "running",
    startedAt: Date.now() - 60_000,
    canStop: false,
    tool: bash("gh-agent run watch 99"),
    ...over,
});

const mount = (a: PinnedActivity) => {
    const [act] = createSignal<PinnedActivity | undefined>(a);
    const [no] = createSignal(false);
    return render(() => <ActivityRow activity={act} expanded={no} leaving={no} onToggle={() => {}} onStop={() => {}} onDismiss={() => {}} />);
};

const animation = (c: HTMLElement) => c.querySelector<HTMLElement>(".agent-activity-sigil .activity-animation");

describe("ActivityRow sigil", () => {
    it("a running tool shows its kind of work's animation just before its phrase, not the slash", () => {
        const { container } = mount(activity());
        const summary = container.querySelector(".agent-activity-summary")!;
        const [first, second] = [...summary.children];
        expect(first.classList.contains("agent-activity-sigil")).toBe(true);
        expect(animation(container)?.getAttribute("data-bucket")).toBe("wait");
        expect(second.textContent).toBe("Wait for v0.60.10 to publish");
        expect(first.textContent).not.toContain("$");
        expect(container.querySelector(".agent-spinner-ascii, .agent-activity-spinner")).toBeNull();
    });

    it("a row younger than 3 s shows its own glyph: quick calls don't flicker", () => {
        const { container } = mount(activity({ startedAt: Date.now() - 1000 }));
        expect(animation(container)).toBeNull();
        expect(container.querySelector(".agent-activity-sigil")?.textContent).toBe("$");
    });

    it("a finished tool shows its result glyph, not the animation", () => {
        const { container } = mount(activity({ status: "done" }));
        expect(animation(container)).toBeNull();
        expect(container.querySelector(".agent-activity-sigil")?.textContent).toBe("✓");
    });

    it("a running shell is sorted by its command, and a subagent is delegated work", () => {
        const shell = mount(activity({ kind: "shell", tool: undefined, shell: { cmd: "cargo build --release", log: { chunks: [] } } as unknown as ShellNode }));
        expect(animation(shell.container)?.getAttribute("data-bucket")).toBe("build");
        cleanup();
        const sub = mount(activity({ kind: "subagent", tool: undefined }));
        expect(animation(sub.container)?.getAttribute("data-bucket")).toBe("delegate");
    });

    it("a young shell keeps its own sigil", () => {
        const { container } = mount(activity({ kind: "shell", tool: undefined, startedAt: Date.now() }));
        expect(animation(container)).toBeNull();
        expect(container.querySelector(".agent-activity-sigil")?.textContent).toBe("⟩");
    });
});

describe("bucketOfActivity", () => {
    const of = (over: Partial<PinnedActivity>) => bucketOfActivity(activity(over)).id;

    it("sorts a Bash tool call by its command", () => {
        expect(of({ tool: bash("cargo nextest run -p srv") })).toBe("test");
        expect(of({ tool: bash("git push -u origin x") })).toBe("send");
        expect(of({ tool: bash("sleep 300") })).toBe("hold");
    });

    it("a command with no rule is the fallback", () => {
        expect(of({ tool: bash("./my-script") })).toBe("other");
        expect(of({ tool: undefined })).toBe("other");
    });
});
