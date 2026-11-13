// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Settings › Advanced, pinned as a whole ahead of splitting it into parts: each scenario mounts the real section over
 * a settings object held in a signal, with stand-ins for the number, toggle, key-value and text controls, and logs
 * the section as an indented tree (every control with the props it was given) and, in order, every setting it writes
 * and every callback it makes. The logs are snapshots recorded on the unsplit component; the split must leave them
 * byte for byte. (This pins the markup and the writes; the controls' own behaviour is tested with the controls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    arg: undefined as unknown,
    devMode: false,
}));

vi.mock("@/app/store/global", async () => {
    const { createSignal } = await import("solid-js");
    const [settings, setSettings] = createSignal<unknown>(undefined);
    return { settingsAtom: settings, __setSettings: setSettings };
});
vi.mock("@/app/store/dev-mode", () => ({
    DEV_MODE_KEY: "dev:mode",
    devModeOn: () => h.devMode,
    setDevMode: (v: boolean) => h.log.push(`setDevMode ${v}`),
}));
vi.mock("@/app/element/ui", async (orig) => {
    const actual = await orig<typeof import("@/app/element/ui")>();
    return {
        ...actual,
        TextInput: (props: Record<string, any>) => {
            const { onBlur, ...rest } = props;
            return <input data-stand="TextInput" {...rest} onBlur={onBlur} />;
        },
    };
});
const describeProps = (props: Record<string, unknown>, skip: string[]) => {
    const out: Record<string, string> = {};
    for (const k of Object.keys(props).sort()) {
        if (skip.includes(k)) continue;
        out[k] = typeof props[k] === "function" ? "<fn>" : (JSON.stringify(props[k]) ?? "undefined");
    }
    return JSON.stringify(out);
};
vi.mock("../settings-controls", async (orig) => {
    const actual = await orig<typeof import("../settings-controls")>();
    return {
        ...actual,
        set: (key: string, value: unknown) => h.log.push(`set ${key} ${JSON.stringify(value)}`),
        NumberControl: (props: Record<string, any>) => (
            <div data-stand="NumberControl" data-props={describeProps(props, ["onChange"])}>
                <span data-cb="onChange" onClick={() => props.onChange(h.arg)}>
                    change
                </span>
            </div>
        ),
        ToggleControl: (props: { checked: boolean; onChange: (v: boolean) => void }) => (
            <div data-stand="ToggleControl" data-checked={String(props.checked)}>
                <span data-cb="onChange" onClick={() => props.onChange(!props.checked)}>
                    toggle
                </span>
            </div>
        ),
        KeyValueEditor: (props: { value: Record<string, string>; onChange: (v: Record<string, string>) => void }) => (
            <div data-stand="KeyValueEditor" data-value={JSON.stringify(props.value)}>
                <span data-cb="onChange" onClick={() => props.onChange({ ...props.value, NEW: "1" })}>
                    change
                </span>
            </div>
        ),
    };
});

import * as globalStore from "@/app/store/global";
import { AdvancedSection } from "./advanced-section";

const setSettings = (globalStore as any).__setSettings as (v: unknown) => void;
const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/advanced-section.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href|id|placeholder|min|max|step)$/;
/** The section as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim().replace(/\s+/g, " ");
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    if (el instanceof HTMLInputElement) attrs.push(`value=${JSON.stringify(el.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    // The kit numbers its fields as they are made, so the count depends on the tests that ran before.
    log(dump(host).join("\n").replace(/-cl-\d+/g, "-cl-N"));
};
const flush = async () => {
    for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
};
function mount() {
    const view = render(() => <AdvancedSection />);
    host = view.container;
}
const row = (id: string) => host.querySelector(`#setting-advanced\\.${id}`) as HTMLElement;
const trigger = (id: string, arg?: unknown) => {
    h.arg = arg;
    fireEvent.click(row(id).querySelector("[data-cb]")!);
};
const blurWith = (el: HTMLInputElement, value: string) => {
    el.value = value;
    fireEvent.blur(el);
};
const input = (id: string) => row(id).querySelector("input") as HTMLInputElement;

const CONFIGURED = {
    "term:disablewebgl": true,
    "agent:askquestiontimeoutms": 45000,
    "agent:compactat": 400000,
    "agent:previewskidnotches": 7,
    "agent:container_memory": 4096,
    "agent:container_cpus": 0,
    "model:spend_cap_usd:openrouter": "none",
    "agent:subagentmodel": "opus",
    "agent:exploremodel": "inherit",
    "agent:compacthints": false,
    "agent:overlapnotes": false,
    "agent:retryonreconnect": false,
    "kanban:reminders": false,
    "telemetry:interval": 5,
    "telemetry:numpoints": 300,
    "dnd:enabled": false,
    "dnd:agentinserttoken": false,
    "dnd:concurrency": 3,
    "cmd:env": { EDITOR: "vim", LANG: "C" },
};

beforeEach(() => {
    T.length = 0;
    h.devMode = false;
    setSettings(undefined);
});
afterEach(() => {
    cleanup();
});

describe("AdvancedSection, pinned", () => {
    it("shows every group and setting with its defaults when nothing is set", async () => {
        mount();
        await flush();
        look("defaults");
        await snapshot("defaults");
    });

    it("shows the values the settings hold, and dev mode when it is on", async () => {
        h.devMode = true;
        setSettings(CONFIGURED);
        mount();
        await flush();
        look("configured");
        await snapshot("configured");
    });

    it("follows the settings while it is open", async () => {
        mount();
        await flush();
        setSettings({ "term:disablewebgl": true, "agent:subagentmodel": "haiku", "dnd:concurrency": 2, "cmd:env": { A: "1" } });
        await flush();
        look("after a change");
        setSettings(undefined);
        await flush();
        look("settings gone");
        await snapshot("follows");
    });

    it("writes each toggle's setting with the opposite value", async () => {
        setSettings({ "kanban:reminders": true });
        mount();
        await flush();
        for (const id of ["disable_webgl", "compact_hints", "overlap_notes", "retry_on_reconnect", "kanban_reminders", "enable_file_drop", "insert_reference_token", "dev_mode"]) {
            trigger(id);
        }
        await flush();
        await snapshot("toggles");
    });

    it("writes each number setting in the unit it is stored in", async () => {
        mount();
        await flush();
        trigger("auto_answer_timeout", 12.4);
        trigger("compact_at", 480.6);
        trigger("compact_at", -3);
        trigger("route_spend_cap", 250);
        trigger("route_spend_cap", -5);
        trigger("preview_skid_notches", 4.6);
        trigger("preview_skid_notches", 999);
        trigger("preview_skid_notches", -2);
        trigger("sysinfo_sample_interval", 2);
        trigger("sysinfo_history_length", 64);
        await flush();
        await snapshot("numbers");
    });

    it("writes the model settings on blur, and clears one that is left empty", async () => {
        mount();
        await flush();
        blurWith(input("subagent_model"), "  opus ");
        blurWith(input("subagent_model"), "   ");
        blurWith(input("explore_model"), "claude-haiku-4-5");
        blurWith(input("explore_model"), "");
        await flush();
        await snapshot("models");
    });

    it("writes a container limit as auto, none or a number, and puts back what it can't use", async () => {
        setSettings({ "agent:container_memory": 2048 });
        mount();
        await flush();
        const mem = input("container_memory");
        const cpus = input("container_cpus");
        blurWith(mem, "AUTO");
        blurWith(mem, "");
        blurWith(mem, "0");
        blurWith(mem, "3072");
        blurWith(mem, "lots");
        log(`memory box after "lots": ${mem.value}`);
        blurWith(mem, "-1");
        log(`memory box after "-1": ${mem.value}`);
        blurWith(cpus, "1.5");
        blurWith(cpus, "many");
        log(`cpus box after "many": ${cpus.value}`);
        await flush();
        await snapshot("limits");
    });

    it("writes the upload limit as unlimited or a whole number of at least one", async () => {
        setSettings({ "dnd:concurrency": 4 });
        mount();
        await flush();
        const box = input("max_concurrent_uploads");
        blurWith(box, "");
        blurWith(box, "6");
        blurWith(box, "0");
        blurWith(box, "x");
        blurWith(box, "3.9");
        await flush();
        await snapshot("uploads");
    });

    it("writes the environment variables", async () => {
        setSettings({ "cmd:env": { EDITOR: "vim" } });
        mount();
        await flush();
        trigger("global_env_vars");
        await flush();
        await snapshot("env");
    });
});
