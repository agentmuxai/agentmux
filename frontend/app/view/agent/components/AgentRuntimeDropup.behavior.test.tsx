// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentRuntimeDropup, pinned as a whole before it was split into hook groups: each scenario drives the
 * trigger and the panel (mouse, keyboard, focus, the process report, a failing change, a Codex pane) and
 * logs, in order, what the DOM shows and what reaches `patchRuntime` and `refreshCodexModels`. The logs are
 * snapshots recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { replaceProviderModels } from "../providers";
import type { ProcessRuntime } from "../process-runtime";
import { AgentRuntimeDropup } from "./AgentRuntimeDropup";

const T: string[] = [];
const log = (line: string) => T.push(line);

const [report, setReport] = createSignal<ProcessRuntime | undefined>(undefined);
vi.mock("../process-runtime", async (importOriginal) => ({
    ...(await importOriginal<typeof import("../process-runtime")>()),
    useProcessRuntime: () => report,
}));

const patchRuntime = vi.fn();
vi.mock("../runtime-apply", () => ({
    applyRuntimeChange: vi.fn().mockResolvedValue(undefined),
    patchRuntime: (...args: unknown[]) => patchRuntime(...args),
}));

vi.mock("../providers/codex-models-refresh", () => ({
    refreshCodexModels: () => log("refreshCodexModels()"),
}));

const SNAP = (name: string) => `./__snapshots__/AgentRuntimeDropup.behavior.${name}.txt`;
const flush = async () => {
    for (let i = 0; i < 4; i++) await Promise.resolve();
};

const [meta, setMeta] = createSignal<Record<string, unknown> | undefined>(undefined);
const runtimeMeta = (model: string, permissionMode = "default", effort = "high") => ({
    "agent:runtime": { model, permissionMode, effort },
});

beforeEach(() => {
    T.length = 0;
    patchRuntime.mockReset().mockImplementation(async (...args: unknown[]) => {
        const current = (args[3] as () => unknown)();
        log(
            `patchRuntime(${args[0]}, ${(args[1] as any)?.id}, ${JSON.stringify(args[2])}, ${JSON.stringify(args[4])}) meta=${JSON.stringify(current)}`
        );
        return {};
    });
    setReport(undefined);
    setMeta(undefined);
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    document.body.innerHTML = "";
});

function mount(providerId = "claude", lastReplyModel?: string | null) {
    render(() => (
        <AgentRuntimeDropup
            blockId="block-1"
            blockAtom={() => ({ meta: meta() }) as any}
            providerId={providerId}
            lastReplyModel={lastReplyModel}
        />
    ));
}

const trigger = () => screen.getByRole("button", { name: /Runtime settings/i }) as HTMLButtonElement;

/** What the trigger and the panel show right now, as lines. */
function view(label: string) {
    const t = trigger();
    log(`-- ${label}`);
    log(
        `trigger "${t.textContent}" class=[${t.className}] expanded=${t.getAttribute("aria-expanded")} width=${t.style.width || "-"} border=${t.style.borderLeft}`
    );
    log(`  title: ${t.title}`);
    log(`  aria: ${t.getAttribute("aria-label")}`);
    const status = document.querySelector(".agent-runtime-dropup-drift");
    if (!status) {
        log("  panel: closed");
        return;
    }
    const lines = Array.from(status.querySelectorAll(".agent-runtime-dropup-drift-line")).map((l) => l.textContent);
    const btn = status.querySelector(".agent-runtime-dropup-drift-btn") as HTMLButtonElement;
    log(
        `  status kind=${status.getAttribute("data-kind")} ${JSON.stringify(lines)} restart[hidden=${btn.classList.contains("is-hidden")} disabled=${btn.disabled} tab=${btn.tabIndex}]`
    );
    for (const el of Array.from(document.querySelectorAll(".agent-runtime-dropup-panel [role=listbox] > *"))) {
        const cls = el.className;
        if (cls.includes("agent-runtime-dropup-section")) {
            const note = el.querySelector(".agent-runtime-dropup-section-note");
            log(`  [${el.firstChild?.textContent}]${note ? ` note: ${note.textContent}` : ""}`);
        } else if (cls.includes("agent-runtime-dropup-note")) {
            log(`  note${cls.includes("is-placeholder") ? " (placeholder)" : ""}: ${el.textContent}`);
        } else {
            const desc = el.querySelector(".agent-runtime-dropup-description")?.textContent;
            const mark = [
                el.getAttribute("aria-selected") === "true" ? "current" : "",
                cls.includes("active") ? "active" : "",
                el.getAttribute("aria-disabled") === "true" ? "disabled" : "",
            ]
                .filter(Boolean)
                .join(",");
            log(`  - ${el.querySelector(".label")?.textContent}${desc ? ` / ${desc}` : ""} {${mark}}`);
        }
    }
}

const click = (el: Element) => fireEvent.click(el);
const key = (k: string, init: KeyboardEventInit = {}) => {
    log(`key ${k}${init.ctrlKey ? "+ctrl" : ""}${init.metaKey ? "+meta" : ""}${init.altKey ? "+alt" : ""}`);
    fireEvent.keyDown(document.activeElement ?? document.body, { key: k, ...init });
};
const rowsByLabel = (re: RegExp) =>
    screen.getAllByRole("option").find((o) => re.test(o.querySelector(".label")?.textContent ?? ""))!;
const closeButton = () => screen.getByRole("button", { name: "Close" });

describe("AgentRuntimeDropup behavior", () => {
    it("opens, closes and holds the trigger's width", async () => {
        const sheet = document.createElement("style");
        sheet.textContent = ".agent-runtime-dropup-trigger { width: 123.5px; }";
        document.head.appendChild(sheet);
        try {
            mount();
            trigger().focus();
            view("closed");
            click(trigger());
            view("opened by the trigger");
            click(trigger());
            view("closed by the trigger");
            click(trigger());
            click(closeButton());
            view("closed by the close button");
            click(trigger());
            fireEvent.mouseDown(document.body);
            view("closed by a mousedown outside");
            click(trigger());
            fireEvent.mouseDown(screen.getByRole("listbox"));
            view("a mousedown inside leaves it open");
            fireEvent.mouseDown(trigger());
            view("a mousedown on the trigger leaves it to the click");
            fireEvent.focusIn(trigger());
            view("focus moving within the dropup leaves it open");
            const outside = document.createElement("input");
            document.body.appendChild(outside);
            outside.focus();
            view("closed by focus moving outside");
            fireEvent.keyDown(document.body, { key: "ArrowDown" });
            fireEvent.keyDown(document.body, { key: "Enter" });
            view("a closed panel does not listen to keys");
            await flush();
        } finally {
            sheet.remove();
        }
        await expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP("open-close"));
    });

    it("answers the keyboard only while focus is in the dropup", async () => {
        mount();
        trigger().focus();
        click(trigger());
        view("opened");
        key("ArrowDown");
        key("ArrowDown");
        view("two down");
        key("ArrowUp");
        view("one up");
        for (let i = 0; i < 4; i++) key("ArrowUp");
        view("up past the first wraps to the last");
        key("ArrowDown");
        view("down past the last wraps to the first");
        key("p");
        view("p jumps to Plan");
        key("H");
        view("H jumps to Haiku");
        key("z");
        view("z matches nothing");
        key("p", { ctrlKey: true });
        key("p", { metaKey: true });
        key("p", { altKey: true });
        key("Tab");
        key("F5");
        key("-");
        view("modified and non-letter keys do nothing");
        key("Enter");
        await flush();
        view("Enter applies the active row and keeps the panel open");
        key("ArrowDown");
        key("Enter");
        await flush();
        key("Escape");
        view("Escape closes");
        click(trigger());
        view("reopened: the active row is the current mode again");
        trigger().blur();
        key("ArrowDown");
        key("Enter");
        key("Escape");
        await flush();
        view("keys with focus elsewhere in the page are ignored, the panel still open");
        await expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP("keyboard"));
    });

    it("selects with the mouse", async () => {
        mount();
        click(trigger());
        view("opened");
        const mouseDown = fireEvent.mouseDown(rowsByLabel(/Plan/));
        log(`mousedown on a row is default-prevented: ${!mouseDown}`);
        fireEvent.mouseEnter(rowsByLabel(/Plan/));
        view("hovering Plan");
        click(rowsByLabel(/Plan/));
        await flush();
        view("clicked Plan");
        click(rowsByLabel(/Opus/));
        await flush();
        click(rowsByLabel(/^max/));
        await flush();
        view("clicked Opus and max");
        setMeta(runtimeMeta("haiku"));
        view("Haiku selected: effort rows are shown but cannot be picked");
        fireEvent.mouseEnter(rowsByLabel(/^medium/));
        click(rowsByLabel(/^medium/));
        await flush();
        view("clicking a disabled row does nothing");
        await expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP("mouse"));
    });

    it("says when a change could not be applied, and clears on the next one", async () => {
        mount();
        click(trigger());
        patchRuntime.mockImplementationOnce(async () => {
            log("patchRuntime rejects with an Error");
            throw new Error("rpc down");
        });
        click(rowsByLabel(/Plan/));
        await flush();
        view("after a failed pick");
        patchRuntime.mockImplementationOnce(async () => {
            log("patchRuntime rejects with a string");
            throw "plain text";
        });
        click(rowsByLabel(/Opus/));
        await flush();
        view("after a second failure");
        click(rowsByLabel(/Sonnet/));
        await flush();
        view("after one that works");
        await expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP("apply-errors"));
    });

    it("says when the agent is not running what is selected", async () => {
        vi.useFakeTimers();
        const AGREES: ProcessRuntime = {
            running: true,
            restartPending: false,
            model: "sonnet",
            effort: "high",
            permissionMode: "default",
        };
        mount();
        view("before any report");
        setReport(AGREES);
        await vi.advanceTimersByTimeAsync(2000);
        view("the process agrees");
        setReport({ ...AGREES, model: "opus", effort: "low", permissionMode: "plan" });
        await vi.advanceTimersByTimeAsync(1000);
        view("a difference, before it has held");
        await vi.advanceTimersByTimeAsync(600);
        view("a difference that has held");
        click(trigger());
        view("opened on a difference");
        click(screen.getByRole("button", { name: /Restart to apply/i }));
        await vi.advanceTimersByTimeAsync(0);
        view("after Restart to apply");
        setReport({ ...AGREES, model: undefined, effort: undefined, permissionMode: undefined });
        await vi.advanceTimersByTimeAsync(2000);
        view("running on the CLI defaults");
        setReport({ ...AGREES, effort: "max", restartPending: true });
        await vi.advanceTimersByTimeAsync(10);
        view("a pending difference");
        setReport(AGREES);
        await vi.advanceTimersByTimeAsync(10);
        view("agrees again");
        await expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP("drift"));
    });

    it("shows the running model line", async () => {
        setMeta(runtimeMeta("sonnet"));
        mount("claude", "claude-sonnet-5-5");
        click(trigger());
        view("a reply from the selected family");
        cleanup();
        T.length = 0;
        mount("claude", "claude-opus-5-5");
        click(trigger());
        view("a reply from another family");
        cleanup();
        T.length = 0;
        mount("claude", null);
        click(trigger());
        view("no reply yet");
        cleanup();
        T.length = 0;
        setReport({ running: true, restartPending: false, model: "sonnet", effort: "high", permissionMode: "default" });
        mount("claude", "claude-opus-5-5");
        click(trigger());
        view("a reply from another family under a process that was spawned with the selection");
        setReport({
            running: true,
            restartPending: false,
            model: "sonnet",
            effort: "high",
            permissionMode: "default",
            effectiveModel: "claude-haiku-4-5",
            effectiveEffort: "low",
        } as ProcessRuntime);
        view("the CLI's own report wins over the last reply");
        await expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP("running-note"));
    });

    it("migrates a superseded model id, and keeps the menu usable when that fails", async () => {
        const warn = vi.spyOn(console, "warn").mockImplementation((...a) => log(`console.warn ${a[0]}`));
        setMeta(runtimeMeta("claude-fable-5"));
        patchRuntime.mockImplementationOnce(async () => {
            log("patchRuntime (migration) rejects");
            throw new Error("rpc down");
        });
        mount();
        await flush();
        view("a superseded id, migration failing");
        click(trigger());
        view("opened");
        setMeta(runtimeMeta("claude-fable-5"));
        setMeta(runtimeMeta("sonnet"));
        await flush();
        view("an alias is left alone");
        setMeta(runtimeMeta("some-unknown-model"));
        await flush();
        view("an unrecognised id is left alone");
        setMeta(runtimeMeta(""));
        await flush();
        view("no model named");
        warn.mockRestore();
        await expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP("migration"));
    });

    it("refreshes the Codex model list as the process starts and as the menu opens", async () => {
        replaceProviderModels("codex", [
            { value: "gpt-6-astra", label: "GPT-6-Astra", efforts: ["low", "medium", "ultra"], defaultEffort: "low" },
            { value: "gpt-6-mini", label: "GPT-6-Mini", efforts: ["low"], defaultEffort: "low", default: true },
        ]);
        try {
            setMeta(runtimeMeta("gpt-6-astra", "bypass", "max"));
            mount("codex");
            view("mounted");
            setReport({ running: true, restartPending: false, model: "gpt-6-astra", effort: "low" });
            await flush();
            view("the process reported");
            click(trigger());
            view("opened");
            click(rowsByLabel(/Read only/));
            click(rowsByLabel(/^ultra/));
            await flush();
            view("picked a mode and an effort");
            setMeta(runtimeMeta("gpt-5-gone", "plan", "low"));
            await flush();
            view("a model the account no longer offers moves to the list's default");
            setReport({ running: true, restartPending: false, model: "gpt-6-mini", permissionMode: "workspace-write" });
            view("a second report");
            click(trigger());
            click(trigger());
            view("closed and opened again");
        } finally {
            replaceProviderModels("codex", []);
        }
        await expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP("codex"));
    });

    it("lays out the Mode section for a persistent agent and a pinned definition", async () => {
        setMeta({
            ...runtimeMeta("sonnet", "bypass", "high"),
            "agent:provider_flags": "--model opus --effort low --permission-mode plan",
        });
        mount();
        click(trigger());
        view("a definition pins Opus, low and Plan");
        click(rowsByLabel(/Haiku/));
        await flush();
        view("picking holds");
        await expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP("pinned"));
    });

    it("falls back to the stock model list and the raw value for a provider and a mode it does not know", async () => {
        setMeta(runtimeMeta("sonnet", "mystery-mode", "mystery-effort"));
        mount("no-such-provider");
        view("closed");
        click(trigger());
        view("opened");
        click(rowsByLabel(/Haiku/));
        await flush();
        setMeta(runtimeMeta("claude-old-model", "default", "high"));
        await flush();
        view("a model id that is not in the stock list");
        await expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP("unknown-provider"));
    });
});
